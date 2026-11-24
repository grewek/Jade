use super::table;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::*;

mod input;
pub(super) mod justify;
mod paint;
pub(in crate::text_editor) mod region;
mod render;
mod table_paint;
mod widgets;

use super::Editor;
use super::colors::{InlineColors, callout_role};
use super::columns::handles::ColumnDrag;
use super::columns::{self, Columns};
use super::doc::DocumentCache;
use super::editor::ColumnMenu;
use super::image::{self, Cache as ImageCache, Info as ImageInfo};
use super::layout::{
    LineState, MathPart, TableLineKind, char_budget, entry_heights, indent_px, is_table_active,
    line_runs, mono_char_width, shape_display_line, shape_entry, shaped_height, table_line_kind,
};
use super::link_table::LinkTable;
use super::paint_trace::{FrameInfo, PaintTrace};
use super::preview::LineRole;
use super::preview::callout::{Callout, callout_index_at};
use super::preview::{
    DisplayLine, InlineStyle, LineIndent, Widget, build_cell_line_with, is_thematic_break,
    raw_cell_line,
};
use super::rows::{Change, Inputs, RowCache};
use super::table::Align;
use super::table::handles::{Grid, Handle, TableDrag};
use super::table::ops::Op;
use super::table::{Table, table_index_at};
use crate::debug_server::targets::DebugName;
use crate::theme::{Alpha, Role, ThemeEngine};
#[allow(unused_imports)]
pub(super) use input::*;
#[allow(unused_imports)]
pub(super) use paint::*;
use region::{paint_control, paint_stacked_controls, restore_pill};
#[allow(unused_imports)]
pub(super) use render::*;
#[allow(unused_imports)]
pub(super) use table_paint::*;
#[allow(unused_imports)]
pub(super) use widgets::*;

/// Pixel width of a callout's left accent bar — thicker than a plain
/// blockquote's own `QUOTE_BAR_WIDTH`, so a callout reads as a distinct,
/// heavier element even before its background tint is noticed.
const CALLOUT_BAR_WIDTH: Pixels = px(3.);

/// The left inset, within an indent level's own lane, that a blockquote's
/// quote bar is painted at — see `paint_lines`.
const QUOTE_BAR_INSET: Pixels = px(4.);
const QUOTE_BAR_WIDTH: Pixels = px(2.);

/// The thickness of a table's grid lines — see `paint_table_grid_lines`.
const TABLE_LINE_WIDTH: Pixels = px(1.);

/// How many lines of overscan (shaped/painted even though just outside the
/// viewport) `visible_line_range` pads the true viewport range by — a
/// smooth-scroll/wrap-estimate-slop margin, not load-bearing for
/// correctness (every line still gets exactly one entry in [`LineLayout`]
/// regardless).
const OVERSCAN_LINES: usize = 20;

/// `origin_y` plus `rows` rows of `line_height`, summed in `f64`.
fn window_y(origin_y: Pixels, line_height: Pixels, rows: i64) -> Pixels {
    let doc = f64::from(f32::from(line_height)) * rows as f64;
    px((f64::from(f32::from(origin_y)) + doc) as f32)
}

/// Per-document, per-paint layout data that lets `measure_document`/
/// `paint_lines`/`offset_for_position`/`input_handler::bounds_for_range`
/// each touch only the lines they actually need, instead of every line in
/// the document — see this module's own notes on viewport-bounded
/// rendering. Built once per paint by `analyze_line_layout`, which first
/// makes sure the two caches below are up to date (cheaply, in the common
/// case — see [`RowCache`]'s own doc comment) and then just wraps
/// clones of them (`Rc`, so cheap) alongside `line_height`; every query
/// method below is an O(log n) lookup into one of them, not an array index,
/// but that's still a tiny cost against what re-deriving them would be.
///
/// Holds the caches themselves rather than, say, a flat `Vec` of resolved
/// byte ranges/heights for the same reason the old flat-`Vec` version of
/// this struct held byte *ranges* rather than borrowed `&str` slices:
/// `render_editor`'s paint closure needs to move this into a `'static`
/// closure, and an owned `Rc` clone satisfies that the same way an owned
/// `Range<usize>` did.
#[derive(Clone)]
pub(super) struct LineLayout {
    doc_cache: Rc<RefCell<DocumentCache>>,
    row_cache: Rc<RefCell<RowCache>>,
    line_height: Pixels,
}

impl LineLayout {
    pub(super) fn line_count(&self) -> usize {
        self.doc_cache.borrow().line_count()
    }

    /// Estimated height of line `i` alone.
    pub(super) fn height(&self, i: usize) -> Pixels {
        self.cumulative_at(i + 1) - self.cumulative_at(i)
    }

    pub(super) fn line_range(&self, i: usize) -> std::ops::Range<usize> {
        let doc = self.doc_cache.borrow();
        doc.line_start_byte(i)..doc.line_end_byte(i)
    }

    /// The estimated document-space Y offset where line `i` begins —
    /// `line_count()` (one past the last line) is valid too, giving the
    /// document's total estimated height.
    pub(super) fn cumulative_at(&self, i: usize) -> Pixels {
        self.line_height * self.row_cache.borrow().prefix_sum(i) as f32
    }

    /// The window y of line `i`'s top, with the editor's top at `origin_y`.
    /// Both are huge in a huge note (a 105 MB one is 57M px tall, where `f32`
    /// steps are 4px), so the sum is done in `f64`: the result is a small number
    /// and exact. Adding `origin_y` to [`Self::cumulative_at`] in `f32` lands
    /// each box and each line on a different 4px step (#21).
    pub(super) fn screen_y(&self, origin_y: Pixels, i: usize) -> Pixels {
        window_y(
            origin_y,
            self.line_height,
            self.row_cache.borrow().prefix_sum(i),
        )
    }

    /// For the paint trace; see [`RowCache::trace_state`].
    pub(super) fn trace_state(&self) -> (Option<Pixels>, std::ops::Range<usize>, usize) {
        self.row_cache.borrow().trace_state()
    }

    /// Brings the row counts up to date for `inp` (see [`Change::Layout`]);
    /// whether they were rebuilt.
    pub(super) fn update_rows(&self, inp: &Inputs, window: &mut Window) -> bool {
        self.row_cache
            .borrow_mut()
            .update(&self.doc_cache, Change::Layout, inp, window)
    }

    /// Makes the row counts of `range` exact; see [`RowCache::verify`].
    pub(super) fn verify_rows(
        &self,
        range: std::ops::Range<usize>,
        inp: &Inputs,
        window: &mut Window,
    ) {
        self.row_cache
            .borrow_mut()
            .verify(&self.doc_cache.borrow(), range, inp, window);
    }

    /// [`Self::verify_rows`] for the lines in view, and how far that moved the
    /// content under the reader: the rows of lines above the first visible one
    /// are corrected when they enter the window, which moves everything below
    /// them. Subtracting the result from the editor's origin and the scroll
    /// offset keeps the content where it was (#24). Zero when the window had
    /// no such corrections.
    pub(super) fn verify_anchored(
        &self,
        (bounds, viewport): (Bounds<Pixels>, Bounds<Pixels>),
        inp: &Inputs,
        window: &mut Window,
    ) -> Pixels {
        let range = visible_line_range(self, bounds, viewport);
        if range.is_empty() {
            return px(0.);
        }
        let doc_top = viewport.origin.y - bounds.origin.y;
        let first = self
            .partition_point_cumulative(|c| c <= doc_top)
            .saturating_sub(1);
        let before = self.row_cache.borrow().prefix_sum(first);
        self.verify_rows(range, inp, window);
        let after = self.row_cache.borrow().prefix_sum(first);
        // `f64`: the rows times the row height is exact where `f32` cumulative
        // heights are not (see `window_y`).
        px((f64::from(f32::from(self.line_height)) * (after - before) as f64) as f32)
    }

    /// The line containing buffer byte offset `cursor`.
    pub(super) fn line_at(&self, cursor: usize) -> usize {
        self.doc_cache.borrow().line_at(cursor)
    }

    /// Upgrades line `i`'s cached row count to `real_rows`; see
    /// [`RowCache::correct_late`]. `&self`: mutates through `row_cache`'s
    /// `RefCell`, like [`Self::cumulative_at`] reads through it.
    pub(super) fn correct_row_count(&self, i: usize, real_rows: usize) {
        self.row_cache.borrow_mut().correct_late(i, real_rows);
    }

    /// Binary search over [`Self::cumulative_at`], matching `[Pixels]::
    /// partition_point`'s semantics (the first index, in `0..=line_count()`,
    /// for which `pred` is false) exactly — reimplemented here rather than
    /// materializing the cumulative sequence into a slice (which would
    /// reintroduce the O(document) cost incremental caching exists to
    /// remove), since `RowCache`'s own `Fenwick` tree only supports
    /// O(log n) *point* queries directly, not slice-style binary search.
    fn partition_point_cumulative(&self, pred: impl Fn(Pixels) -> bool) -> usize {
        let mut lo = 0usize;
        let mut hi = self.line_count();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if pred(self.cumulative_at(mid)) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// Where the caret at `offset` is on screen (window coordinates): in a region's
/// column, else in its plain line. `None` when its line is outside the viewport.
pub(super) fn caret_rect(
    text: &str,
    doc: &DocumentCache,
    layout: &LineLayout,
    (offset, cursor, focused): (usize, usize, bool),
    (font, font_size, text_color, line_height): (&Font, Pixels, Hsla, Pixels),
    (bounds, viewport): (Bounds<Pixels>, Bounds<Pixels>),
    window: &mut Window,
) -> Option<Bounds<Pixels>> {
    let range = visible_line_range(layout, bounds, viewport);
    if range.is_empty() {
        return None;
    }
    if let Some(found) = caret_in_column(
        text,
        doc,
        layout,
        (offset, cursor, focused),
        (font, font_size, line_height),
        (text_color.into(), bounds),
        window,
    ) {
        return Some(found);
    }
    let anchor = layout.screen_y(bounds.origin.y, range.start);
    let mut y_offset = px(0.);
    for i in range {
        let line_range = layout.line_range(i);
        let line_text = &text[line_range.clone()];
        let state = LineState::of(doc, i, cursor, focused);
        let results = shape_display_line(
            line_text,
            &state,
            font,
            font_size,
            text_color,
            bounds.size.width,
            window,
        );
        // More than one entry only for a table row whose cells wrapped.
        let heights = entry_heights(&results, line_text, (font_size, line_height), state.macros);
        for ((wrapped, display_line), entry) in results.iter().zip(heights) {
            let (line_height, height) = (entry.row, entry.height);
            if offset <= line_range.end {
                let display_offset = display_line.display_offset_for_raw(offset - line_range.start);
                let pos = justify::position_for_index(
                    (&state, wrapped, display_line),
                    results.len(),
                    display_offset,
                    line_height,
                )
                .unwrap_or(point(px(0.), px(0.)));
                let x = pos.x + indent_px(display_line.indent.level());
                return Some(Bounds {
                    origin: point(bounds.origin.x + x, anchor + y_offset + pos.y),
                    size: size(px(1.), line_height),
                });
            }
            y_offset += height;
        }
    }
    None
}

/// Up or Down by one painted row, keeping the caret's x: a wrapped paragraph's
/// rows, and the rows of a float's text, are stepped one by one instead of the
/// whole source line. `false` (nothing moved) when the last frame is stale, the
/// target is off the editor, or the move would not change the caret, so the
/// caller moves by source line as before.
pub(super) fn move_by_row(
    editor: &mut Editor,
    down: bool,
    extend: bool,
    window: &mut Window,
) -> bool {
    let Some(view) = editor.last_view.clone() else {
        return false;
    };
    let cursor = editor.buffer().cursor();
    let caret = {
        let doc = editor.document_cache.borrow();
        if view.cursor != cursor || !view.focused || view.layout.line_count() != doc.line_count() {
            return false;
        }
        caret_rect(
            editor.buffer().text(),
            &doc,
            &view.layout,
            (cursor, cursor, true),
            (
                &view.font,
                view.font_size,
                view.text_color,
                view.line_height,
            ),
            (view.bounds, view.viewport),
            window,
        )
    };
    let Some(caret) = caret else {
        return false;
    };
    let goal = match editor.visual_goal {
        Some((at, x)) if at == cursor => x,
        _ => caret.origin.x,
    };
    let lh = view.line_height;
    // The row below starts under the caret's own (possibly tall) row.
    let y = caret.origin.y + lh * 0.5 + if down { caret.size.height } else { -lh };
    let (top, bottom) = (
        view.bounds.origin.y,
        view.bounds.origin.y + view.bounds.size.height,
    );
    if y < top || y >= bottom {
        return false;
    }
    // The row may be just outside the viewport: look a few rows further.
    let pad = lh * 3.;
    let wide = FrameView {
        viewport: Bounds {
            origin: view.viewport.origin - point(px(0.), pad),
            size: size(
                view.viewport.size.width,
                view.viewport.size.height + pad * 2.,
            ),
        },
        ..(*view).clone()
    };
    let target = hit_for_position(editor, &wide, point(goal, y), window).offset;
    if (down && target <= cursor) || (!down && target >= cursor) {
        return false;
    }
    editor.buffer.set_cursor(target, extend);
    editor.visual_goal = Some((target, goal));
    editor.skip_hidden_lines(down, extend);
    true
}

/// The width regions are laid out across for an editor `available_width`
/// wide: none before the first layout (no real width yet).
/// See [`region::caret_bounds`].
pub(super) fn caret_in_column(
    text: &str,
    doc: &DocumentCache,
    layout: &LineLayout,
    (offset, cursor, focused): (usize, usize, bool),
    metrics: (&Font, Pixels, Pixels),
    (colors, bounds): (InlineColors, Bounds<Pixels>),
    window: &mut Window,
) -> Option<Bounds<Pixels>> {
    let (region, _) = doc.side_by_side(doc.line_at(offset))?;
    // The region's top as a window y, so nothing here adds two huge numbers.
    let at_top = Bounds {
        origin: point(
            bounds.origin.x,
            layout.screen_y(bounds.origin.y, region.start_line),
        ),
        size: bounds.size,
    };
    region::caret_bounds(
        text,
        doc,
        offset,
        (cursor, focused),
        metrics,
        (colors, px(0.), at_top),
        window,
    )
}

/// Makes sure `row_cache` is up to date (see [`Change::Layout`]) and returns a
/// [`LineLayout`] wrapping it alongside `doc_cache`.
pub(super) fn analyze_line_layout(
    doc_cache: &Rc<RefCell<DocumentCache>>,
    row_cache: &Rc<RefCell<RowCache>>,
    inp: &Inputs,
    line_height: Pixels,
    window: &mut Window,
) -> LineLayout {
    row_cache
        .borrow_mut()
        .update(doc_cache, Change::Layout, inp, window);
    LineLayout {
        doc_cache: doc_cache.clone(),
        row_cache: row_cache.clone(),
        line_height,
    }
}

/// The `[start, end)` line-index range actually relevant to paint/hit-test
/// this frame — the viewport's own document-space Y-range (`bounds`, the
/// editor's own on-screen bounds, vs. `viewport`, the scrollable
/// container's fixed on-screen bounds — same two values `autoscroll_to_cursor`
/// already compares), found via binary search (`partition_point_cumulative`,
/// matching `[Pixels]::partition_point`'s own semantics) rather than a
/// linear walk, padded by [`OVERSCAN_LINES`] on each side and clamped to
/// the document's own line count. Falls back to the whole document if the
/// viewport hasn't been laid out yet (`viewport.size.height <= 0`, e.g. the
/// very first paint before `scroll_handle`'s tracked container has a size)
/// — matches `autoscroll_to_cursor`'s own "nothing to do yet" guard.
pub(super) fn visible_line_range(
    layout: &LineLayout,
    bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
) -> std::ops::Range<usize> {
    let line_count = layout.line_count();
    if line_count == 0 {
        return 0..0;
    }
    if viewport.size.height <= px(0.) {
        return 0..line_count;
    }
    let doc_top = viewport.origin.y - bounds.origin.y;
    let doc_bottom = doc_top + viewport.size.height;
    let start = layout
        .partition_point_cumulative(|c| c <= doc_top)
        .saturating_sub(1);
    let end = layout.partition_point_cumulative(|c| c < doc_bottom);
    let start = start.saturating_sub(OVERSCAN_LINES);
    let end = (end + OVERSCAN_LINES).min(line_count);
    start..end
}

/// Everything a frame's painting and mouse hit-testing share, captured once
/// at paint time: the line layout and where the editor, its viewport and the
/// cursor are, with the font the lines are shaped in.
#[derive(Clone)]
pub(super) struct FrameView {
    layout: LineLayout,
    cursor: usize,
    focused: bool,
    bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    line_height: Pixels,
    font: Font,
    font_size: Pixels,
    text_color: Hsla,
    /// The table controls shown and the grip being dragged, as of this frame.
    table_hover: Option<(usize, Vec<Handle>)>,
    table_drag: Option<TableDrag>,
    /// The columns region whose controls show, and the divider being dragged.
    column_hover: Option<usize>,
    column_drag: Option<ColumnDrag>,
    /// See `Editor::float_spots`.
    float_spots: Rc<RefCell<Vec<(usize, Vec<region::FloatSpot>)>>>,
}

/// What painting a table needs about the columns it is drawn with.
pub(super) struct TableColumns {
    pub(super) widths: Vec<usize>,
    pub(super) aligns: Vec<table::Align>,
    pub(super) char_width: Pixels,
    /// The width the box is stretched to: the whole of its room when the table
    /// had to shrink to fit (so its edge lines up with a callout above or below
    /// it), else 0 for the natural width.
    fill: Pixels,
}

impl TableColumns {
    pub(super) fn of(
        table: &Table,
        width: Pixels,
        (font, font_size, text_color): (&Font, Pixels, Hsla),
        window: &mut Window,
    ) -> Self {
        let char_width = mono_char_width(font, font_size, text_color, window);
        let max_chars = char_budget(width, char_width);
        Self {
            widths: table::resolve_widths(&table.columns, max_chars),
            aligns: table.columns.iter().map(|c| c.align).collect(),
            char_width,
            fill: if max_chars.is_some_and(|m| !table::fits_unshrunk(&table.columns, m)) {
                width
            } else {
                px(0.)
            },
        }
    }

    fn x(&self, chars: f32) -> Pixels {
        self.char_width * chars
    }

    /// The box's width: its columns', or its room where the table shrank.
    pub(super) fn box_px(&self) -> Pixels {
        self.x(table::box_width(&self.widths)).max(self.fill)
    }
}

/// The boxes of the tables in `range` that render as tables (not raw while the
/// cursor is in them): header tint, column dividers, row separators and the
/// outline, all from the row cache's heights, before any text so the cells go
/// on top. Heights past the verified window are estimates, but that is off
/// screen.
#[allow(clippy::too_many_arguments)]
/// Where a table box is and its row and column boundaries, as painted.
fn table_grid(
    view: &FrameView,
    doc: &DocumentCache,
    text: &str,
    table: &Table,
    window: &mut Window,
) -> Option<(Grid, Point<Pixels>)> {
    let FrameView {
        layout,
        bounds,
        font,
        font_size,
        text_color,
        line_height,
        ..
    } = view;
    let cfg = (font, *font_size, *text_color);
    if let Some((region, split)) = doc.side_by_side(table.start_line) {
        let shaped = region::shape_region(
            text,
            doc,
            (&region, &split),
            (view.cursor, view.focused),
            (font, *font_size, *line_height),
            ((*text_color).into(), None),
            window,
        );
        let column = region::column_with_line(&shaped, table.start_line)?;
        // The table is as wide as the line that opens it.
        let first = column.lines.iter().find(|sl| sl.line == table.start_line)?;
        let region_top = layout.screen_y(bounds.origin.y, region.start_line);
        let columns = TableColumns::of(table, first.width, cfg, window);
        let at = |line: usize| region::line_top(column, region_top, line);
        let (grid, top) = grid_for(table, &columns, &at);
        return Some((grid, point(bounds.origin.x + first.x, top)));
    }
    let columns = TableColumns::of(table, bounds.size.width, cfg, window);
    let at = |line: usize| layout.screen_y(bounds.origin.y, line);
    let (grid, top) = grid_for(table, &columns, &at);
    Some((grid, point(bounds.origin.x, top)))
}

/// A table's grid for `at`, the window y of a line's top, and the window y of
/// the table's top.
fn grid_for(table: &Table, columns: &TableColumns, at: &dyn Fn(usize) -> Pixels) -> (Grid, Pixels) {
    let top = at(table.start_line);
    let boundary = |line: usize| f32::from(at(line) - top);
    // The header and its delimiter band are row 0.
    let row_y = [0.0, boundary(table.start_line + 2)]
        .into_iter()
        .chain((table.start_line + 3..=table.end_line).map(boundary))
        .collect();
    let mut col_x: Vec<f32> = (0..=columns.widths.len())
        .map(|c| f32::from(columns.x(table::column_left(&columns.widths, c))))
        .collect();
    if let Some(last) = col_x.last_mut() {
        *last = f32::from(columns.box_px());
    }
    (Grid { col_x, row_y }, top)
}

// Deliberately not `use super::*`/`use gpui_kit::*` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows plain `#[test]` and makes `#[gpui_kit::test]`'s own expansion
// recurse infinitely (see the project-wide note on this in `app.rs`).
#[cfg(test)]
mod math_rows_tests;

#[cfg(test)]
mod anchoring_tests;
#[cfg(test)]
mod formula_context_tests;
#[cfg(test)]
mod height_agreement_tests;

#[cfg(test)]
mod tests {
    use super::super::Editor;
    use super::super::preview::LineIndent;
    use super::super::preview::LineRole;
    use super::super::table::classify_tables;
    use super::super::test_support::apply_edit;
    use super::LineState;
    use super::window_y;
    use super::{TableLineKind, shape_display_line};
    use crate::text_editor::layout::is_active_line;
    use crate::text_editor::rows::{self as rowcache, Change, Inputs, Invalidate};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, EntityInputHandler, Pixels, Point, TestAppContext,
        WindowOptions, point, px,
    };

    fn open_editor(
        cx: &mut TestAppContext,
        text: &'static str,
    ) -> (AnyWindowHandle, Entity<Editor>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::ThemeEngine::install(None, cx);
        });
        cx.update(|cx| {
            // Must out-rank `gpui_kit::init`'s own global "tab" binding —
            // see `Editor::handle_insert_tab`'s doc comment.
            cx.bind_keys(crate::text_editor::key_bindings());
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

    #[test]
    fn window_y_stays_exact_in_a_note_millions_of_pixels_tall() {
        // 1,358,363 rows of 26px end at y=35,317,438, which `f32` (4px steps
        // up here) cannot hold: summing in `f32` gives -8 instead of -10.
        let (origin, lh, rows) = (px(-35_317_448.), px(26.), 1_358_363);
        assert_eq!(window_y(origin, lh, rows), px(-10.));
        assert_ne!(origin + lh * rows as f32, px(-10.));
        // Neighbouring lines keep their whole-row spacing.
        let step = window_y(origin, lh, rows + 1) - window_y(origin, lh, rows);
        assert_eq!(step, lh);
    }

    #[gpui_kit::test]
    fn height_scales_with_line_count(cx: &mut TestAppContext) {
        // Both taller than the window: the editor is never shorter than its
        // container, so short texts would all measure as the container.
        let (one_line_window, _editor) =
            open_editor(cx, Box::leak(vec!["line"; 200].join("\n").into_boxed_str()));
        let one_line_height = cx
            .update_window(one_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        let (three_line_window, _editor) =
            open_editor(cx, Box::leak(vec!["line"; 600].join("\n").into_boxed_str()));
        let three_line_height = cx
            .update_window(three_line_window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        assert_eq!(
            three_line_height,
            one_line_height * 3.,
            "three times the lines should be exactly 3x as tall"
        );
    }

    #[gpui_kit::test]
    fn shape_display_line_wraps_at_the_given_width(cx: &mut TestAppContext) {
        // A direct, low-level check of the wrapping mechanism itself
        // (`shape_display_line`'s `wrap_width_for` dispatch into
        // `WindowTextSystem::shape_text`), independent of the *rest* of
        // `app.rs`'s `scrolling_away_from_the_cursor_does_not_get_snapped_back`-
        // style tests, which check the surrounding sizing/autoscroll
        // plumbing instead. A bare `Editor` has no tracked ancestor to
        // measure wrapping against for its *own* div sizing (see
        // `app.rs`'s `a_line_too_wide_for_the_pane_wraps_instead_of_overflowing`,
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
                    &line,
                    &LineState::plain(),
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            assert_eq!(
                unwrapped.size(line_height).height,
                line_height,
                "an effectively unbounded width should never wrap"
            );

            let (wrapped, _) = {
                let mut results = shape_display_line(
                    &line,
                    &LineState::plain(),
                    &font,
                    font_size,
                    text_style.color,
                    px(100.),
                    window,
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
    fn shape_display_line_never_gpui_wraps_a_table_row_even_at_a_narrow_width(
        cx: &mut TestAppContext,
    ) {
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
                "| Alice | 30 |",
                &LineState {
                    table_kind: TableLineKind::Row(table),
                    ..LineState::plain()
                },
                &font,
                font_size,
                text_style.color,
                px(10.),
                window,
            );
            for (wrapped, _) in &results {
                assert_eq!(
                    wrapped.size(line_height).height,
                    line_height,
                    "no individual entry should itself further (GPUI-level) wrap"
                );
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn shape_display_line_wraps_a_table_rows_cells_within_their_own_column(
        cx: &mut TestAppContext,
    ) {
        // The actual fix: a cell too wide for the editor's own width wraps
        // *within its column* (multiple `shape_display_line` entries for
        // the one raw row) rather than letting the row overflow past the
        // editor's right edge.
        let text =
            "| Name | Bio |\n| --- | --- |\n| Alice | A very long biography that does not fit |";
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
                row,
                &LineState {
                    table_kind: TableLineKind::Row(table),
                    ..LineState::plain()
                },
                &font,
                font_size,
                text_style.color,
                px(150.),
                window,
            );
            assert!(
                narrow.len() > 1,
                "a cell too wide for a 150px budget should split into multiple sub-rows"
            );

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
                row,
                &LineState {
                    table_kind: TableLineKind::Row(table),
                    ..LineState::plain()
                },
                &font,
                font_size,
                text_style.color,
                Pixels::MAX,
                window,
            );
            assert_eq!(
                wide.len(),
                1,
                "an effectively unbounded width should never need to wrap a cell"
            );
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

        let focused = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").focused()
            })
            .unwrap();
        assert_eq!(focused, Some(true), "clicking the editor should focus it");

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert!(
            cursor <= 2,
            "clicking near the start should place the cursor near byte 0, got {cursor}"
        );
    }

    /// A click on a word of a justified row (drawn shifted right of where the
    /// unstretched text would put it) lands on that word, through real dispatch.
    #[gpui_kit::test]
    fn clicking_a_justified_word_lands_on_it(cx: &mut TestAppContext) {
        let text = "The quick brown fox jumps over the lazy dog and then keeps running \
            through the quiet meadow until the evening settles over the hills again. "
            .repeat(8);
        let text: &'static str = Box::leak(text.trim_end().to_owned().into_boxed_str());
        let (window, editor) = open_editor(cx, text);
        cx.update(|cx| cx.set_global(crate::text_editor::TextSettings { justify: true }));
        let (bounds, target, x) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("editor").bounds();
                let style = window.text_style();
                let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
                let state = LineState {
                    justify: true,
                    ..LineState::plain()
                };
                let mut shaped = shape_display_line(
                    text,
                    &state,
                    &font,
                    size,
                    style.color,
                    bounds.size.width,
                    window,
                );
                let (wrapped, display) = shaped.remove(0);
                let rows = wrapped.wrap_boundaries().len();
                assert!(rows >= 1, "the paragraph must wrap in the test window");
                // A word in the middle of row 0.
                let target = text[..text.len() / (rows + 1) / 2].rfind(' ').unwrap() + 1;
                let at = super::justify::position_for_index(
                    (&state, &wrapped, &display),
                    1,
                    target,
                    window.line_height(),
                )
                .unwrap();
                let natural = wrapped
                    .position_for_index(target, window.line_height())
                    .unwrap();
                assert!(at.x > natural.x + px(1.), "the row should be stretched");
                (bounds, target, at.x)
            })
            .unwrap();
        cx.update_window(window, |_, window, cx| {
            window.click_at("editor", point(x + px(1.), px(1.)), cx);
        })
        .unwrap();
        let _ = bounds;
        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        assert_eq!(cursor, target, "the click should land on the word it hit");
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
        assert_eq!(
            text, "hello\t",
            "tab should insert a tab character, not fall through to a no-op"
        );

        let focused = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").focused()
            })
            .unwrap();
        assert_eq!(
            focused,
            Some(true),
            "tab must not move focus out of the editor"
        );
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
        assert_eq!(
            text, "hello world",
            "pasting should bring the cut text back"
        );
    }

    #[gpui_kit::test]
    fn ctrl_z_undoes_a_whole_burst_of_typed_characters(cx: &mut TestAppContext) {
        // `window.input` sends one char at a time through
        // `replace_text_in_range`, the real typing path.
        let (window, editor) = open_editor(cx, "hello");
        cx.update_window(window, |_, window, cx| {
            focus_editor(window, cx);
            window.press("end", cx);
            window.input(" world", cx);
            window.press("ctrl-z", cx);
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(text, "hello", "the typed burst undoes as one step");
    }

    #[gpui_kit::test]
    fn ctrl_z_then_ctrl_shift_z_round_trips_a_pasted_insert(cx: &mut TestAppContext) {
        // Pasted text arrives in one `buffer.insert` call: a single step.
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
        assert_eq!(
            text, "hello",
            "ctrl-z should undo the whole pasted insert in one step"
        );

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
        assert_eq!(
            marked_preview, "^",
            "the dead-key preview should appear immediately"
        );

        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.replace_text_in_range(None, "^", window, cx);
            });
        })
        .unwrap();

        let text = editor.read_with(cx, |editor, _| editor.buffer().text().to_string());
        assert_eq!(
            text, "^",
            "committing the composition must replace the preview, not duplicate it"
        );
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
        assert_eq!(
            selection,
            Some(0..5),
            "ctrl-shift-right should select through the end of \"hello\""
        );
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
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(text.len(), false)
        });
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
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(text.len(), false)
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renders_callouts_without_panicking(cx: &mut TestAppContext) {
        let text = concat!(
            "> [!info]\n",
            "> Some additional information.\n",
            "\n",
            "> [!warning]+ Custom Title\n",
            "> body with a nested quote:\n",
            ">> [!note]\n",
            ">> this nested header is just plain nested-blockquote text\n",
            "\n",
            "active line"
        );
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(text.len(), false)
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_a_previewed_heading_lands_past_its_hidden_marker_and_activates_it(
        cx: &mut TestAppContext,
    ) {
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
            // (Mid-row: the corner itself belongs to the table's `+` control.)
            window.click_at("editor", point(px(1.), line_height * 1.5), cx);
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
        let active = editor.read_with(cx, |editor, _| {
            is_active_line(&(11..text.len()), editor.buffer().cursor(), true)
        });
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
            // (Mid-row: the corner itself belongs to the table's `+` control.)
            window.click_at("editor", point(px(1.), line_height * 1.5), cx);
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
        // on top of `preview`'s own exhaustive pure-logic tests.
        let text = "1. outer\n   - inner\n     still inner, lazy continuation\n> quoted\nlazy quote continuation";
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| {
            editor.buffer_mut().set_cursor(text.len(), false)
        });
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
        let long_bio: &'static str = Box::leak(
            format!(
                "| Name | Bio |\n| --- | --- |\n| Alice | {} |",
                "word ".repeat(100)
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, long_bio);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_table_renders_as_a_box_until_the_cursor_enters_it_then_renders_raw(
        cx: &mut TestAppContext,
    ) {
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
                    header,
                    &LineState {
                        table_kind: TableLineKind::Row(table),
                        ..LineState::plain()
                    },
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            // A rendered row is empty text: `paint_table_row` draws its cells and
            // `paint_table_boxes` the box over it.
            assert_eq!(grid.display_text, "");

            let (_, raw) = {
                let mut results = shape_display_line(
                    header,
                    &LineState {
                        table_kind: TableLineKind::Row(table),
                        table_active: true,
                        ..LineState::plain()
                    },
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            assert_eq!(
                raw.display_text, header,
                "once the cursor is anywhere in the table, it must render raw"
            );
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
            // (Mid-row: the corner itself belongs to the table's `+` control.)
            window.click_at("editor", point(px(1.), line_height * 1.5), cx);
        })
        .unwrap();

        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        let name_start = text.find("Name").unwrap();
        assert_eq!(
            cursor, name_start,
            "clicking near the table's left border should land at the first cell's content"
        );
    }

    #[gpui_kit::test]
    fn fenced_code_content_is_never_inline_scanned_even_on_the_active_line(
        cx: &mut TestAppContext,
    ) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let line = "**not bold** [[not a wikilink]]";
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            // `active: true` on its own would normally be enough to show a
            // line verbatim anyway (see `preview`'s own tests) — the
            // real thing this test guards is that `FenceContent` forces the
            // same literal treatment regardless of `active`, since a fenced
            // code line must never have its content inline-scanned at all,
            // active or not.
            let (_, display_line) = {
                let mut results = shape_display_line(
                    line,
                    &LineState {
                        role: LineRole::FenceContent,
                        active: true,
                        ..LineState::plain()
                    },
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            assert_eq!(
                display_line.display_text, line,
                "fenced code content must never be treated as Markdown"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn fenced_code_content_starting_with_quote_or_list_syntax_is_never_indented(
        cx: &mut TestAppContext,
    ) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            for line in ["> looks like a quote", "- looks like a list item"] {
                let (_, display_line) = {
            let mut results = shape_display_line(line, &LineState { role: LineRole::FenceContent, ..LineState::plain() }, &font, font_size, text_style.color, Pixels::MAX, window);
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

    /// A click in a laid-out region lands in the column nearest it, and in
    /// that column's line under it. (The headless editor never learns a real
    /// width, so this drives the region's shaping and hit-test directly.)
    /// The table controls work on a table inside a column: they show when the
    /// pointer is over that table, a press on one is found, and nothing shows
    /// over the other column.
    #[gpui_kit::test]
    fn table_controls_work_on_a_table_in_a_column(cx: &mut TestAppContext) {
        use super::super::table::handles::Handle;
        use super::px;
        use super::{FrameView, LineLayout, RowCache, handle_for_pointer, hover_for_pointer};
        use gpui::{Bounds, size};
        use std::{cell::RefCell, rc::Rc};

        let text = "intro\n\n::: columns\n\nleft\n\n---\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n:::\n\nafter";
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let (text_color, line_height) = (text_style.color, window.line_height());
            let width = px(800.);
            let doc_cache = editor.read(cx).document_cache.clone();
            doc_cache.borrow_mut().view.set_column_width(Some(800.));
            let mut rows = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut rows,
                text,
                &doc_cache.borrow(),
                (0, true),
                width,
                (&font, font_size, text_color),
                window,
            );
            let layout = LineLayout {
                doc_cache: doc_cache.clone(),
                row_cache: Rc::new(RefCell::new(rows)),
                line_height,
            };
            let bounds = Bounds {
                origin: point(px(100.), px(50.)),
                size: size(width, px(500.)),
            };
            let view = FrameView {
                layout,
                cursor: 0,
                focused: true,
                bounds,
                viewport: bounds,
                line_height,
                font: font.clone(),
                font_size,
                text_color,
                table_hover: None,
                table_drag: None,
                column_hover: None,
                column_drag: None,
                float_spots: Default::default(),
            };
            let split = doc_cache.borrow().side_by_side(2).unwrap().1;
            // The table is the right column's first block, at the region's top
            // (the region starts on row 2).
            let top = bounds.origin.y + line_height * 2.;
            let at = |x: f32, y: Pixels| point(bounds.origin.x + px(x), top + y);
            let over_table = at(split.x(1) + 20., line_height * 1.5);
            let over_left = at(split.x(0) + 20., line_height * 1.5);
            let editor = editor.read(cx);

            let hover = hover_for_pointer(editor, &view, over_table, window)
                .expect("controls show over the table in the right column");
            assert_eq!(hover.0, 0, "the region's table is the document's first");
            assert!(!hover.1.is_empty());
            assert!(hover_for_pointer(editor, &view, over_left, window).is_none());
            // Far above the region: nothing.
            assert!(
                hover_for_pointer(editor, &view, at(split.x(1) + 20., px(-60.)), window).is_none()
            );

            // A press on a shown grip finds it, at the grip's own place.
            let grip = hover
                .1
                .iter()
                .find(|h| matches!(h, Handle::ColumnGrip(_)))
                .copied()
                .expect("a column grip is shown");
            let doc = doc_cache.borrow();
            let (grid, origin) =
                super::table_grid(&view, &doc, text, &doc.tables()[0], window).unwrap();
            let (x, y, w, h) = super::table::handles::rect_of(&grid, grip);
            let centre = origin + point(px(x + w / 2.), px(y + h / 2.));
            assert_eq!(
                handle_for_pointer(editor, &view, centre, window),
                Some((0, grip))
            );
        })
        .unwrap();
    }

    /// Floats on both sides leave the text between them; where a side's floats
    /// end the text widens, cutting the paragraph at that row, and floats stacked
    /// down one side keep a row between them.
    #[gpui_kit::test]
    fn text_flows_between_floats_and_widens_as_each_side_ends(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;

        let para = "flowing words ".repeat(60);
        let text: &'static str = Box::leak(
            format!(
                "::: aside left 25%\n\nshort left\n\n--- right 25%\n\nright one\n\nright one b\n\n--- right 25%\n\nright two\n\n---\n\n{para}\n\n:::\n"
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1400.));
            let (region, split) = doc.side_by_side(0).expect("an aside lays out");
            assert_eq!(region.floats().len(), 3);
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            );
            let text_line = region.column_lines(3).start;
            let flow: Vec<_> = shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .filter(|sl| sl.line == text_line)
                .collect();
            assert!(flow.len() >= 3, "cut where each side ends: {}", flow.len());
            // Narrow between both floats, then only the right one, then all.
            assert_eq!(flow[0].x, px(split.x(3)));
            assert_eq!(flow[0].width, px(split.width(3)));
            assert_eq!(flow[1].x, px(0.));
            assert!(flow[1].width > flow[0].width && flow[1].width < px(1400.));
            assert_eq!(flow.last().unwrap().width, px(1400.));
            // The pieces tile the raw line.
            for pair in flow.windows(2) {
                assert_eq!(pair[0].raw.end, pair[1].raw.start);
            }
            assert_eq!(flow.last().unwrap().raw.end, para.len());
            // Two floats on the right are stacked, a row between them.
            let right_one = shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .find(|sl| text[doc.line_start_byte(sl.line)..].starts_with("right one b"));
            let right_two = shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .find(|sl| text[doc.line_start_byte(sl.line)..].starts_with("right two"));
            let (one, two) = (right_one.unwrap(), right_two.unwrap());
            assert_eq!(two.y, one.y + one.height + lh, "a row of air between");
            assert_eq!(one.x, two.x);
        })
        .unwrap();
    }

    /// A cold rebuild counts a float block's region as a region (floats' stacks
    /// plus text past them), close to its shaped height, so the content height
    /// (and the scrollbar) hardly moves as paint verifies it; counted as plain
    /// lines at full width it was half again too tall.
    #[gpui_kit::test]
    fn a_cold_rebuild_estimates_a_float_block_near_its_shaped_height(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::RowCache;
        use super::region::shape_region;
        let para = "flowing words ".repeat(40);
        let text: &'static str = Box::leak(
            format!(
                "::: float right 30%\n\nfloat words here\n\n:::\n\n{para}\n\n{para}\n\n{para}\n\nafter"
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1000.));
            let mut rows = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut rows,
                text,
                &doc,
                (0, false),
                px(1000.),
                (&font, size, style.color),
                window,
            );
            let (region, split) = doc.side_by_side(0).unwrap();
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            )
            .rows as f32;
            let estimate = rows.prefix_sum(1) as f32;
            assert!(
                (estimate - shaped).abs() <= shaped * 0.25,
                "estimated {estimate} rows, shaped {shaped}"
            );
        })
        .unwrap();
    }

    /// A table that had to shrink to its room fills it, so its edge lines up
    /// with a callout in the same column; one that fits keeps its natural width.
    #[gpui_kit::test]
    fn a_shrunk_table_fills_its_room(cx: &mut TestAppContext) {
        use super::super::table::classify_tables;
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let text = "| a very long heading | another long heading |\n| - | - |\n| x | y |\n";
            let table = classify_tables(text).1.remove(0);
            let narrow =
                super::TableColumns::of(&table, px(170.), (&font, size, style.color), window);
            assert_eq!(narrow.box_px(), px(170.));
            let wide =
                super::TableColumns::of(&table, px(2000.), (&font, size, style.color), window);
            assert!(wide.box_px() < px(2000.));
        })
        .unwrap();
    }

    /// A float block keeps a row of air below its lowest float, so what follows
    /// the region (another float block, a table) never touches it.
    #[gpui_kit::test]
    fn a_float_block_taller_than_its_text_keeps_a_row_of_air(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;
        let tall = "float line\n\n".repeat(8);
        let text: &'static str =
            Box::leak(format!("::: float right 30%\n\n{tall}:::\n\nshort").into_boxed_str());
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1400.));
            let (region, split) = doc.side_by_side(0).expect("a float block lays out");
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            );
            let spot = shaped.float_spots[0];
            let bottom = spot.top + spot.height;
            assert!(
                lh * shaped.rows as f32 >= bottom + lh,
                "{} rows, the float ends at {bottom:?}",
                shaped.rows
            );
        })
        .unwrap();
    }

    /// A float block has no wrapper: the paragraphs after it flow beside it
    /// and, once it ends, run at the whole width, all inside its one region.
    #[gpui_kit::test]
    fn a_float_block_flows_the_paragraphs_after_it(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;
        let para = "flowing words ".repeat(80);
        let text: &'static str = Box::leak(
            format!(
                "::: float right 25%\n\nfloat one\n\nfloat two\n\n:::\n\n{para}\n\nsecond\n\nthird"
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1400.));
            let (region, split) = doc.side_by_side(0).expect("a float block lays out");
            assert!(region.tail);
            assert_eq!(
                region.end_line,
                doc.line_count(),
                "the text runs to the end"
            );
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            );
            let lines = |line: usize| {
                shaped
                    .columns
                    .iter()
                    .flat_map(|c| &c.lines)
                    .filter(|sl| sl.line == line)
                    .collect::<Vec<_>>()
            };
            let first = region.column_lines(1).start;
            let beside = lines(first);
            // The paragraph starts in the channel beside the float, and where the
            // float ends it is cut and goes on at the whole width.
            assert_eq!(beside[0].x, px(0.));
            assert!(beside[0].width < px(1400.));
            assert!(beside.len() >= 2, "cut where the float ends");
            assert_eq!(beside.last().unwrap().width, px(1400.));
            // The pieces tile the raw line, and the paragraphs after it follow
            // at the whole width, in this region.
            assert_eq!(beside.last().unwrap().raw.end, para.len());
            let last = region.column_lines(1).end - 1;
            assert_eq!(lines(last)[0].width, px(1400.));
            assert!(lines(last)[0].y > beside.last().unwrap().y);
            assert_eq!(doc.region_index_at(last), Some(0));
        })
        .unwrap();
    }

    /// A second float block joins the first's region: written after some text it
    /// is put down below that text, on the other side beside it, and the text
    /// flows between the two. On the same side it waits below the first.
    #[gpui_kit::test]
    fn a_later_float_is_placed_where_the_text_reaches_it(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;
        let para = "flowing words ".repeat(60);
        let text: &'static str = Box::leak(
            format!(
                "::: float left 20%\n\nA one\n\nA two\n\n:::\n\n{para}\n\n::: float right 20%\n\nB one\n\nB two\n\n:::\n\n::: float left 20%\n\nC\n\n:::\n\n{para}\n"
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1400.));
            let (region, split) = doc.side_by_side(0).expect("the floats lay out");
            assert_eq!(region.count, 4, "three floats share one region");
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            );
            let first_y = |prefix: &str| {
                shaped
                    .columns
                    .iter()
                    .flat_map(|c| &c.lines)
                    .find(|sl| text[doc.line_start_byte(sl.line)..].starts_with(prefix))
                    .map(|sl| (sl.x, sl.y))
                    .unwrap_or_else(|| panic!("{prefix} is laid out"))
            };
            let (a_x, a_y) = first_y("A one");
            let (b_x, b_y) = first_y("B one");
            let (c_x, c_y) = first_y("C");
            assert_eq!(a_y, px(0.), "the first float is at the top");
            assert!(a_x < b_x, "B is on the right");
            // B is down below the paragraph's first rows, not at the top.
            assert!(b_y > px(0.), "B waits for the text that precedes it");
            // C is on A's side and waits for A to end.
            assert_eq!(c_x, a_x);
            assert!(c_y >= a_y + lh * 3., "C is below A");
            // The text goes on beside whichever floats are down.
            let widths: Vec<_> = shaped
                .columns
                .iter()
                .flat_map(|c| &c.lines)
                .filter(|sl| text[doc.line_start_byte(sl.line)..].starts_with("flowing"))
                .map(|sl| sl.width)
                .collect();
            assert!(widths.iter().any(|&w| w < px(1400.)));
            assert!(widths.iter().any(|&w| w == px(1400.)));
        })
        .unwrap();
    }

    /// `::: clear` starts the text below it under every float above it.
    #[gpui_kit::test]
    fn a_clear_marker_moves_the_text_below_the_floats(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;
        let tall = "float line\n\n".repeat(6);
        let text: &'static str = Box::leak(
            format!(
                "::: float right 30%\n\n{tall}:::\n\nbefore the clear\n\n::: clear\n\nafter the clear\n\nlast"
            )
            .into_boxed_str(),
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(1400.));
            let (region, split) = doc.side_by_side(0).expect("a float block lays out");
            assert_eq!(region.clears().len(), 1);
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, lh),
                (style.color.into(), None),
                window,
            );
            let at = |prefix: &str| {
                shaped
                    .columns
                    .iter()
                    .flat_map(|c| &c.lines)
                    .find(|sl| text[doc.line_start_byte(sl.line)..].starts_with(prefix))
                    .map(|sl| (sl.y, sl.width))
                    .unwrap_or_else(|| panic!("{prefix} is laid out"))
            };
            let (before_y, before_w) = at("before the clear");
            let (after_y, after_w) = at("after the clear");
            let float_bottom = shaped
                .columns
                .iter()
                .find(|c| c.x > px(700.))
                .map(|c| c.lines.last().map(|l| l.y + l.height).unwrap());
            assert!(before_w < px(1400.), "beside the float");
            assert!(before_y < float_bottom.unwrap());
            assert!(
                after_y >= float_bottom.unwrap(),
                "below the float: {after_y:?}"
            );
            assert_eq!(after_w, px(1400.));
            assert_eq!(doc.region_index_at(region.clears()[0]), Some(0));
        })
        .unwrap();
    }

    /// Up and Down step one painted row, so a wrapped paragraph beside a float
    /// is walked row by row (not left for the float's last line), keeping the
    /// caret's x, and the row below the cut is reached from the one above it.
    #[gpui_kit::test]
    fn up_and_down_step_painted_rows(cx: &mut TestAppContext) {
        let para = "flowing words ".repeat(40);
        let text: &'static str = Box::leak(
            format!(
                "::: aside right 40%\n\nfloat one\n\nfloat two\n\nfloat three\n\n---\n\n{para}\n\n:::\n\nafter"
            )
            .into_boxed_str(),
        );
        let (window, editor) = open_editor(cx, text);
        let key = |cx: &mut TestAppContext, k: &str| {
            cx.update_window(window, |_, window, cx| {
                window.dispatch_keystroke(gpui_kit::Keystroke::parse(k).unwrap(), cx);
                window.render_frame(cx);
            })
            .unwrap();
        };
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.document_cache
                    .borrow_mut()
                    .view
                    .set_column_width(Some(800.));
                let at = text.find("flowing").unwrap() + 3;
                e.buffer_mut().set_cursor(at, false);
                e.focus_handle.focus(window, cx);
            });
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();
        let start = text.find("flowing").unwrap();
        let end = start + para.len();
        let cursor = |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().cursor());
        let mut seen = vec![cursor(cx)];
        for _ in 0..12 {
            key(cx, "down");
            seen.push(cursor(cx));
        }
        // Each press moves on, inside the paragraph while it has rows, and by
        // roughly one row of text (about 30 to 60 bytes in the narrow channel).
        let inside: Vec<_> = seen
            .iter()
            .filter(|&&c| (start..=end).contains(&c))
            .collect();
        assert!(
            inside.len() >= 4,
            "walks several rows of the paragraph: {seen:?}"
        );
        for pair in inside.windows(2) {
            assert!(pair[1] > pair[0], "moves forward: {seen:?}");
            assert!(pair[1] - pair[0] < 120, "one row, not the line: {seen:?}");
        }
        // And back up, row by row, never into the float.
        let top = *inside.last().unwrap();
        editor.update(cx, |e, _| e.buffer_mut().set_cursor(*top, false));
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        key(cx, "up");
        let up = cursor(cx);
        assert!(
            up < *top && *top - up < 120 && up >= start,
            "up one row: {up}"
        );
    }

    /// Up and Down walk a side-by-side region in source order and never leave
    /// the caret on a marker or padding line; the keys that would join a
    /// column to one do nothing.
    /// A region's controls show over it and just above it, a press finds a
    /// grip, and dragging it gives the snapped shares.
    #[gpui_kit::test]
    fn region_controls_hover_press_and_drag(cx: &mut TestAppContext) {
        use super::super::columns;
        use super::super::columns::handles::ColumnDrag;
        use super::super::table::handles::Handle;
        use super::px;
        use super::{FrameView, LineLayout, RowCache};
        use super::{column_drag_percent, column_handle_for_pointer, column_hover_for_pointer};
        use gpui::{Bounds, size};
        use std::{cell::RefCell, rc::Rc};

        let text = "intro\n\n::: columns\n\nleft\n\n---\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n:::\n\nafter";
        let (window, editor) = open_editor(cx, text);
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let (text_color, line_height) = (text_style.color, window.line_height());
            let width = px(800.);
            let doc_cache = editor.read(cx).document_cache.clone();
            doc_cache.borrow_mut().view.set_column_width(Some(800.));
            let mut rows = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut rows,
                text,
                &doc_cache.borrow(),
                (0, true),
                width,
                (&font, font_size, text_color),
                window,
            );
            let layout = LineLayout {
                doc_cache: doc_cache.clone(),
                row_cache: Rc::new(RefCell::new(rows)),
                line_height,
            };
            let bounds = Bounds {
                origin: point(px(100.), px(50.)),
                size: size(width, px(500.)),
            };
            let view = FrameView {
                layout,
                cursor: 0,
                focused: true,
                bounds,
                viewport: bounds,
                line_height,
                font: font.clone(),
                font_size,
                text_color,
                table_hover: None,
                table_drag: None,
                column_hover: None,
                column_drag: None,
                float_spots: Default::default(),
            };
            let split = doc_cache.borrow().side_by_side(2).unwrap().1;
            // The table is the right column's first block, at the region's top
            // (the region starts on row 2).
            let top = bounds.origin.y + line_height * 2.;
            let at = |x: f32, y: Pixels| point(bounds.origin.x + px(x), top + y);
            let over_table = at(split.x(1) + 20., line_height * 1.5);
            let over_left = at(split.x(0) + 20., line_height * 1.5);

            let editor = editor.read(cx);
            assert_eq!(column_hover_for_pointer(editor, &view, over_left), Some(0));
            assert_eq!(column_hover_for_pointer(editor, &view, over_table), Some(0));
            let above = at(split.x(1), px(-10.));
            assert_eq!(column_hover_for_pointer(editor, &view, above), Some(0));
            let far = at(split.x(1), px(-60.));
            assert_eq!(column_hover_for_pointer(editor, &view, far), None);

            // A press on the divider's grip finds it.
            let doc = doc_cache.borrow();
            let region = doc.columns()[0];
            let frame = columns::handles::Frame {
                dividers: vec![split.divider_x(0)],
                width: split.x(1) + split.width(1),
                height: f32::from(view.layout.height(region.start_line)),
                can_add: true,
                buttons: true,
                grips: Vec::new(),
            };
            let (x, y, w, h) =
                columns::handles::rect_of(&frame, columns::handles::Handle::Divider(0));
            let grip = at(x + w / 2., px(y + h / 2.));
            assert_eq!(
                column_handle_for_pointer(editor, &view, grip),
                Some((0, columns::handles::Handle::Divider(0)))
            );

            // Dragging it to the right of the middle gives a wider left column.
            let drag = ColumnDrag {
                region: 0,
                divider: 0,
                percent: vec![50, 50],
            };
            let to = at(split.x(1) + 100., px(10.));
            let percent = column_drag_percent(editor, &view, &drag, to, window).unwrap();
            assert!(percent[0] > 50 && percent[0] % 5 == 0 && percent[0] + percent[1] == 100);

            // Dragged far enough to stack the region (a column under 160px), the
            // drag is not lost: moving the pointer back brings the columns back,
            // and the stacked region offers a button to restore them.
            let stacked = vec![90, 10];
            drop(doc);
            doc_cache
                .borrow_mut()
                .view
                .set_column_preview(Some((0, stacked.clone())));
            let doc = doc_cache.borrow();
            assert!(doc.side_by_side(2).is_none(), "the preview stacks");
            let drag = ColumnDrag {
                region: 0,
                divider: 0,
                percent: stacked,
            };
            let back = column_drag_percent(editor, &view, &drag, at(split.x(1), px(10.)), window)
                .expect("a stacked preview can still be dragged");
            assert!((40..=60).contains(&back[0]), "{back:?}");
            let (pill, shares) = super::restore_pill(&view, &doc, 0).expect("a restore button");
            assert_eq!(shares, vec![50, 50]);
            drop(doc);
            let center = pill.origin + point(pill.size.width / 2., pill.size.height / 2.);
            assert_eq!(
                super::restore_pill_at(editor, &view, center),
                Some((0, vec![50, 50]))
            );
            assert!(super::restore_pill_at(editor, &view, at(0., px(500.))).is_none());
        })
        .unwrap();
    }

    /// Up and Down walk a side-by-side region in source order and never leave
    /// the caret on a marker or padding line; the keys that would join a
    /// column to one do nothing.
    #[gpui_kit::test]
    fn the_caret_steps_over_a_regions_hidden_lines(cx: &mut TestAppContext) {
        let text = "intro\n\n::: columns\n\nleft\n\n---\n\nright\n\n:::\n\nafter";
        let (_window, editor) = open_editor(cx, text);
        let line_of = |cx: &mut TestAppContext| {
            editor.read_with(cx, |e, _| {
                e.document_cache.borrow().line_at(e.buffer().cursor())
            })
        };
        editor.update(cx, |e, _| {
            e.document_cache
                .borrow_mut()
                .view
                .set_column_width(Some(800.));
            let at = text.find("left").unwrap() + 2;
            e.buffer_mut().set_cursor(at, false);
        });
        editor.update(cx, |e, _| e.move_vertical(true, false));
        assert_eq!(line_of(cx), 8, "down from left enters the right column");
        editor.update(cx, |e, _| e.move_vertical(false, false));
        assert_eq!(line_of(cx), 4, "up from right returns to the left");
        editor.update(cx, |e, _| {
            e.move_vertical(true, false);
            e.move_vertical(true, false);
        });
        assert_eq!(
            line_of(cx),
            11,
            "down from the last column leaves the region"
        );
        editor.update(cx, |e, _| {
            e.move_vertical(true, false);
            e.move_vertical(false, false);
            e.move_vertical(false, false);
        });
        assert_eq!(line_of(cx), 8, "up from below enters the last column");
        // Home of the first column line: Backspace must not eat the padding.
        editor.update(cx, |e, _| {
            let start = e.document_cache.borrow().line_start_byte(4);
            e.buffer_mut().set_cursor(start, false);
            assert!(e.at_region_edge(true));
            let end = e.document_cache.borrow().line_end_byte(8);
            e.buffer_mut().set_cursor(end, false);
            assert!(e.at_region_edge(false));
            e.buffer_mut().set_cursor(end - 2, false);
            assert!(!e.at_region_edge(false));
        });
    }

    /// The structure edits on regions are one undo step each and give back
    /// valid regions.
    #[gpui_kit::test]
    fn structure_edits_make_change_and_unwrap_a_region(cx: &mut TestAppContext) {
        use super::{ColumnMenu, Context, Window};
        let text = "intro\n\nfirst block\nsecond line\n\nafter";
        let (window, editor) = open_editor(cx, text);
        let text_of =
            |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().text().to_owned());
        let run =
            |cx: &mut TestAppContext,
             f: &dyn Fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool| {
                cx.update_window(window, |_, window, cx| {
                    editor.update(cx, |e, cx| f(e, window, cx))
                })
                .unwrap()
            };
        editor.update(cx, |e, _| {
            e.buffer_mut()
                .set_cursor(text.find("second").unwrap(), false)
        });
        assert!(editor.read_with(cx, |e, _| e.column_menu_state() == Some(ColumnMenu::Block)));
        assert!(run(cx, &|e, w, c| e.make_columns(w, c)));
        assert_eq!(
            text_of(cx),
            "intro\n\n::: columns\n\nfirst block\nsecond line\n\n---\n\n\n\n:::\n\nafter"
        );
        assert!(editor.read_with(cx, |e, _| matches!(
            e.column_menu_state(),
            Some(ColumnMenu::Region { can_add: true })
        )));
        assert!(run(cx, &|e, w, c| e.set_region_ratio(
            0,
            vec![60, 40],
            w,
            c
        )));
        assert!(text_of(cx).contains("::: columns 60/40\n"));
        assert!(run(cx, &|e, w, c| e.add_region_column(0, w, c)));
        assert!(
            text_of(cx)
                .contains("::: columns\n\nfirst block\nsecond line\n\n---\n\n\n\n---\n\n\n\n:::")
        );
        assert!(run(cx, &|e, w, c| e.unwrap_region(0, w, c)));
        assert_eq!(text_of(cx), "intro\n\nfirst block\nsecond line\n\nafter");
        // Each edit was one undo step.
        for _ in 0..4 {
            cx.update_window(window, |_, window, cx| {
                editor.update(cx, |e, cx| {
                    let edit = e.buffer_mut().undo().expect("an edit to undo");
                    e.record_edit(edit, window, cx);
                })
            })
            .unwrap();
        }
        assert_eq!(text_of(cx), text);
    }

    /// Regression for issue #10: the second column "Make columns" (and the
    /// float menu) creates was unreachable, a click there landed in the first
    /// column's padding. A click on it must put the caret on a line of that
    /// column, where typing goes.
    #[gpui_kit::test]
    fn a_click_reaches_the_empty_column_a_menu_made(cx: &mut TestAppContext) {
        let wrapped = crate::text_editor::columns::ops::wrapped(&["some text"]).join("\n");
        let floated = crate::text_editor::columns::ops::floated(&["f"], &[], true).join("\n");
        // The float is on the right, so its text is on the left.
        for (source, x, column) in [(wrapped, 0.8, 1), (floated, 0.2, 1)] {
            let text: &'static str = Box::leak(source.into_boxed_str());
            let (window, editor) = open_editor(cx, text);
            editor.update(cx, |e, _| {
                e.document_cache
                    .borrow_mut()
                    .view
                    .set_column_width(Some(800.));
            });
            let width = cx
                .update_window(window, |_, window, cx| {
                    window.render_frame(cx);
                    window.find("editor").bounds().size.width
                })
                .unwrap();
            // Level with the region's first row.
            cx.update_window(window, |_, window, cx| {
                window.click_at("editor", point(width * x, px(4.)), cx);
            })
            .unwrap();
            let on_column = editor.read_with(cx, |e, _| {
                let doc = e.document_cache.borrow();
                let line = doc.line_at(e.buffer().cursor());
                doc.side_by_side(line)
                    .and_then(|(region, _)| region.column_of(line))
            });
            assert_eq!(on_column, Some(column), "in {text:?}");
        }
    }

    /// A float block's menu actions: flip rewrites one marker, move swaps the
    /// block with the neighbouring paragraph, unwrap leaves its content as a
    /// paragraph; each is one undo step.
    #[gpui_kit::test]
    fn a_float_block_flips_moves_and_unwraps(cx: &mut TestAppContext) {
        use super::{ColumnMenu, Context, Window};
        let text = "::: float right 30%\n\nF\n\n:::\n\none\n\ntwo\n\nthree";
        let (window, editor) = open_editor(cx, text);
        let text_of =
            |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().text().to_owned());
        let run =
            |cx: &mut TestAppContext,
             f: &dyn Fn(&mut Editor, &mut Window, &mut Context<Editor>) -> bool| {
                cx.update_window(window, |_, window, cx| {
                    editor.update(cx, |e, cx| f(e, window, cx))
                })
                .unwrap()
            };
        let at = text_of(cx).find("two").unwrap();
        editor.update(cx, |e, _| e.buffer_mut().set_cursor(at, false));
        assert!(editor.read_with(cx, |e, _| e.column_menu_state() == Some(ColumnMenu::Float)));
        assert!(run(cx, &|e, w, c| e.flip_float(w, c)));
        assert!(text_of(cx).starts_with("::: float left 30%\n"));
        assert!(run(cx, &|e, w, c| e.move_float(true, w, c)));
        assert_eq!(
            text_of(cx),
            "one\n\n::: float left 30%\n\nF\n\n:::\n\ntwo\n\nthree"
        );
        // Nothing before "one" to move past.
        let at = text_of(cx).find("two").unwrap();
        editor.update(cx, |e, _| e.buffer_mut().set_cursor(at, false));
        assert!(run(cx, &|e, w, c| e.move_float(false, w, c)));
        assert_eq!(
            text_of(cx),
            "::: float left 30%\n\nF\n\n:::\n\none\n\ntwo\n\nthree"
        );
        assert!(!run(cx, &|e, w, c| e.move_float(false, w, c)));
        assert!(run(cx, &|e, w, c| e.unwrap_float(w, c)));
        assert_eq!(text_of(cx), "F\n\none\n\ntwo\n\nthree");
    }

    /// Resizing a float block rewrites only its open markers (not the text that
    /// flows around it) and is one undo step; it keeps each float's side.
    #[gpui_kit::test]
    fn resizing_a_float_block_rewrites_just_its_markers(cx: &mut TestAppContext) {
        let text = "::: float right 30%\n\nA\n\n:::\n\none\n\n::: float left 20%\n\nB\n\n:::\n\ntwo\n\n::: float right 30%\n\nC\n\n:::\n\nthree";
        let (window, editor) = open_editor(cx, text);
        let text_of =
            |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().text().to_owned());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            editor.update(cx, |e, cx| {
                let region = e.document_cache.borrow().columns()[0];
                assert_eq!(region.floats().len(), 3, "one region");
                // The right floats take 25%, the left one is kept.
                assert!(e.set_region_ratio(0, vec![25, 20, 25, 0], window, cx));
            })
        })
        .unwrap();
        assert_eq!(
            text_of(cx),
            text.replace("right 30%", "right 25%"),
            "only the markers change"
        );
        // Nothing to do when the shares are as they are.
        let same = cx
            .update_window(window, |_, window, cx| {
                editor.update(cx, |e, cx| {
                    e.set_region_ratio(0, vec![25, 20, 25, 0], window, cx)
                })
            })
            .unwrap();
        assert!(!same);
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| {
                let edit = e.buffer_mut().undo().expect("an edit to undo");
                e.record_edit(edit, window, cx);
            })
        })
        .unwrap();
        assert_eq!(text_of(cx), text, "one undo step");
    }

    #[gpui_kit::test]
    fn a_click_in_a_region_lands_in_the_nearest_columns_line(cx: &mut TestAppContext) {
        use super::super::{columns, doc::DocumentCache};
        use super::region::{HitFrame, hit, shape_region};
        use gpui::{Bounds, Window, size};

        let text = "intro\n\n::: columns\n\nleft text\n\n---\n\nright text\n\n:::\n\nafter";
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let line_height = window.line_height();
            let mut doc = DocumentCache::new(text);
            doc.view.set_column_width(Some(800.));
            let (region, split) = doc.side_by_side(2).expect("side by side at 800px");
            let shaped = shape_region(
                text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, font_size, line_height),
                (text_style.color.into(), None),
                window,
            );
            assert_eq!(shaped.rows, 1, "each column holds one short line");
            assert_eq!(shaped.columns.len(), 2);

            let bounds = Bounds {
                origin: point(px(100.), px(50.)),
                size: size(px(800.), px(500.)),
            };
            let top = line_height * 2.;
            let click = |x: f32, window: &mut Window| {
                let position = point(
                    bounds.origin.x + px(x),
                    bounds.origin.y + top + line_height / 2.,
                );
                let frame = HitFrame {
                    bounds,
                    relative_y: position.y - bounds.origin.y,
                    position,
                    line_height,
                    font: &font,
                    font_size,
                    text_color: text_style.color,
                };
                let hit = hit(
                    &doc,
                    text,
                    &shaped,
                    &region,
                    (top, (0, false)),
                    &frame,
                    window,
                );
                doc.line_at(hit.offset)
            };
            assert_eq!(click(split.x(0) + 4., window), 4, "left column");
            assert_eq!(click(split.x(1) + 4., window), 8, "right column");
            // The gap is 24px: 4px short of the right column is nearer it, and 4px
            // past the left column's end is nearer the left one.
            assert_eq!(
                click(split.x(1) - 4., window),
                8,
                "the gap, nearer the right"
            );
            assert_eq!(
                click(split.x(1) - columns::GAP + 4., window),
                4,
                "the gap, nearer the left"
            );
            assert_eq!(click(799., window), 8, "the far right edge");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_columns_marker_line_shows_nothing_unless_active(cx: &mut TestAppContext) {
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            for line in ["::: columns 60/40", ":::"] {
                for active in [false, true] {
                    let results = shape_display_line(
                        line,
                        &LineState {
                            column_frame: true,
                            active,
                            ..LineState::plain()
                        },
                        &font,
                        font_size,
                        text_style.color,
                        Pixels::MAX,
                        window,
                    );
                    assert_eq!(results.len(), 1, "one row either way: {line:?}");
                    let shown = &results[0].1.display_text;
                    let expected = if active { line } else { "" };
                    assert_eq!(shown, expected, "{line:?} active={active}");
                }
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
                    line,
                    &LineState {
                        role: LineRole::FenceDelimiter,
                        ..LineState::plain()
                    },
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            assert_eq!(
                display_line.display_text, "",
                "an inactive fence delimiter line should show nothing"
            );

            let (_, active_display_line) = {
                let mut results = shape_display_line(
                    line,
                    &LineState {
                        role: LineRole::FenceDelimiter,
                        active: true,
                        ..LineState::plain()
                    },
                    &font,
                    font_size,
                    text_style.color,
                    Pixels::MAX,
                    window,
                );
                assert_eq!(results.len(), 1, "expected exactly one entry");
                results.remove(0)
            };
            assert_eq!(
                active_display_line.display_text, line,
                "the active fence delimiter line should show its raw text"
            );
        })
        .unwrap();
    }

    /// Every line's current row count, read out of `rows` via its own
    /// Fenwick-backed prefix sums — what every assertion below compares
    /// between an incrementally-patched cache and an exact oracle.
    fn row_counts(rows: &super::RowCache) -> Vec<i64> {
        (0..rows.line_count())
            .map(|i| rows.prefix_sum(i + 1) - rows.prefix_sum(i))
            .collect()
    }

    /// The true, never-estimated row count for every line, computed
    /// directly via `row_count_for_line` (the same function `RowCache
    /// ::patch_lines`/`sync_active_line` already always use) — the oracle
    /// every property test below compares an incrementally-maintained
    /// cache against. Deliberately *not* `RowCache::rebuild`'s own
    /// output: `rebuild` itself now estimates for non-table lines (see its
    /// own doc comment) and is only guaranteed exact for whichever single
    /// line is currently active, so it stopped being a valid full-document
    /// "ground truth" the moment that landed — this helper is what still
    /// is one.
    fn exact_row_counts(
        text: &str,
        doc: &super::super::doc::DocumentCache,
        (cursor, focused): (usize, bool),
        width: Pixels,
        (font, font_size, text_color): (&gpui_kit::Font, Pixels, gpui_kit::Hsla),
        window: &mut gpui_kit::Window,
    ) -> Vec<i64> {
        let inp = Inputs {
            text,
            cursor,
            focused,
            width,
            font,
            font_size,
            text_color,
        };
        (0..doc.line_count())
            .map(|i| rowcache::exact_row_count(&inp, doc, i, window) as i64)
            .collect()
    }

    /// The property [`super::RowCache::patch_lines`]/[`super::
    /// RowCache::rebuild`] actually rely on: after any sequence of
    /// edits (including line-count-changing ones, which force a fallback
    /// full rebuild — see `needs_full_rebuild`), a cache kept incrementally
    /// in sync edit-by-edit must have exactly the same per-line row counts
    /// as [`exact_row_counts`] computes directly from scratch on the final
    /// text — the same "fuzz against an oracle" discipline `document_
    /// cache.rs`'s own property tests already use, which is what actually
    /// caught every real bug in *that* module; this is the matching test
    /// for the row-height cache this module adds on top of it. Compared
    /// against `exact_row_counts` specifically, not a second `RowCache
    /// ::rebuild` call — `rebuild` itself now also estimates for non-table
    /// lines (see its own doc comment), so it stopped being usable as this
    /// test's oracle the moment that landed; an earlier version of this
    /// test compared against `rebuild` directly and passed only by
    /// coincidence (this text/width combination never happened to make an
    /// estimate differ from the truth), until the *other* property test
    /// below caught the real gap on a text that does. `focused: false`
    /// throughout (no line is ever "active") — active-line transitions are
    /// a deliberately separate, narrower mechanism, covered by its own test
    /// below instead of complicating this one.
    #[gpui_kit::test]
    fn row_height_cache_patch_lines_matches_the_exact_row_counts_after_a_sequence_of_edits(
        cx: &mut TestAppContext,
    ) {
        use super::super::doc::DocumentCache;
        use super::RowCache;

        let mut text = String::from(
            "intro\n```\ncode line one\ncode line two\n```\nplain\n> quote\n- item\n  - nested\nmore plain text here\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |\nafter table\n![pic](pic.png)\n# Heading\nFinal line",
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let available_width = px(400.);

            let doc_cache = std::cell::RefCell::new(DocumentCache::new(&text));
            let mut row_cache = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut row_cache,
                &text,
                &doc_cache.borrow(),
                (0, false),
                available_width,
                (&font, font_size, text_color),
                window,
            );

            // (start, delete_len, insert) — a mix of same-line-count edits
            // (exercising the real `patch_lines` incremental path) and
            // line-count-changing ones (exercising the full-rebuild fallback).
            let edits: &[(usize, usize, &str)] = &[
                (0, 0, "x"),                       // insert at the very start
                (5, 0, "!"),                       // insert mid-line, no newline
                (0, 1, ""),                        // delete one char at the start
                (0, 0, "a new unterminated line"), // same-line-count edit far from any construct
                (9, 0, "\n"),                      // insert a bare newline (line-count change)
                (0, 0, "```\nmore code\n```\n"),   // insert a whole new fence block
                (0, 3, ""),                        // delete across a newline (line-count change)
            ];

            for &(start, delete_len, insert) in edits {
                let old_end = start + delete_len;
                let (new_text, edit) = apply_edit(&text, start, old_end, insert);
                let patch = doc_cache.borrow_mut().record_edit(&new_text, edit);
                let inp = Inputs {
                    text: &new_text,
                    cursor: 0,
                    focused: false,
                    width: available_width,
                    font: &font,
                    font_size,
                    text_color,
                };
                match &patch {
                    Some(patch) => {
                        row_cache.update(&doc_cache, Change::Edit(patch), &inp, window);
                    }
                    None => {
                        row_cache.invalidate(Invalidate::Width);
                        row_cache.update(&doc_cache, Change::Layout, &inp, window);
                    }
                }

                let oracle_doc = DocumentCache::new(&new_text);
                let expected = exact_row_counts(
                    &new_text,
                    &oracle_doc,
                    (0, false),
                    available_width,
                    (&font, font_size, text_color),
                    window,
                );

                assert_eq!(
                    row_counts(&row_cache),
                    expected,
                    "after editing {:?} on text {:?}",
                    (start, old_end, insert),
                    text
                );
                text = new_text;
            }
        })
        .unwrap();
    }

    /// A side-by-side region is one block: its first line holds its tallest
    /// column's rows, its other lines none, and a stacked region (too narrow)
    /// gives every line its own rows. Patching an edit in a column matches a
    /// fresh build.
    #[gpui_kit::test]
    fn a_side_by_side_region_takes_its_tallest_columns_rows(cx: &mut TestAppContext) {
        use super::super::{columns, doc::DocumentCache};
        use super::RowCache;
        use crate::text_editor::layout::mono_char_width;
        use crate::text_editor::layout::wrapped_row_count;
        use gpui::Window;

        let long = "word ".repeat(30);
        let text = format!("intro\n\n::: columns\n\n{long}\n\n---\n\nshort\n\n:::\n\nafter");
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let char_width = mono_char_width(&font, font_size, text_color, window);
            let width = px(600.);
            let build = |doc: &DocumentCache, width: Pixels, window: &mut Window| {
                let mut rows = RowCache::empty();
                rowcache::rebuild_for_test(
                    &mut rows,
                    &text,
                    doc,
                    (0, false),
                    width,
                    (&font, font_size, text_color),
                    window,
                );
                rows
            };

            let mut doc = DocumentCache::new(&text);
            doc.view.set_column_width(Some(f32::from(width)));
            let rows = build(&doc, width, window);
            let column = px(columns::split(f32::from(width), &[50, 50])
                .unwrap()
                .width(0));
            let tall = wrapped_row_count(long.trim_end(), Some(column), char_width);
            assert!(tall > 2, "the long column must wrap, got {tall} rows");
            // Region lines 2..=10: the first holds the height, the rest none.
            assert_eq!(rowcache::count_at(&rows, 2), tall as i64);
            assert!((3..=10).all(|i| rowcache::count_at(&rows, i) == 0));
            assert_eq!(
                (rowcache::count_at(&rows, 0), rowcache::count_at(&rows, 12)),
                (1, 1)
            );

            // Too narrow for 160px columns: stacked, nothing is hidden.
            let narrow = px(300.);
            doc.view.set_column_width(Some(f32::from(narrow)));
            let stacked = build(&doc, narrow, window);
            assert!((0..13).all(|i| rowcache::count_at(&stacked, i) >= 1));

            // An edit in the short column makes it the taller one.
            doc.view.set_column_width(Some(f32::from(width)));
            let mut rows = build(&doc, width, window);
            let at = text.find("short").unwrap();
            let longer = "longer ".repeat(60);
            let (new_text, edit) = apply_edit(&text, at, at + 5, &longer);
            let patch = doc.record_edit(&new_text, edit).unwrap();
            rowcache::patch_for_test(
                &mut rows,
                &new_text,
                &doc,
                patch.touched,
                (0, false),
                (&font, font_size, text_color),
                window,
            );
            let mut fresh = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut fresh,
                &new_text,
                &doc,
                (0, false),
                width,
                (&font, font_size, text_color),
                window,
            );
            let counts = |r: &RowCache| {
                (0..13)
                    .map(|i| rowcache::count_at(&r, i))
                    .collect::<Vec<_>>()
            };
            assert_eq!(counts(&rows), counts(&fresh));
            assert!(
                rowcache::count_at(&rows, 2) > tall as i64,
                "the edited column is now taller"
            );
        })
        .unwrap();
    }

    /// Moving a divider rewrites the region's marker; the rows of a table and a
    /// callout in its columns must come out as a cold rebuild would have them.
    #[gpui_kit::test]
    fn moving_a_divider_relays_out_a_table_and_a_callout(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::RowCache;
        use gpui::Window;

        let note = "> [!note] Title\n> a callout with plenty of words in it that will \
            need to wrap onto several rows when its column is narrow, and more words";
        let table =
            "| Name | Notes |\n| --- | --- |\n| Alice | a long cell with many words to wrap |";
        let para = "plain words ".repeat(20);
        let before = format!(
            "intro\n\n::: columns\n\n{para}\n\n---\n\n{table}\n\n---\n\n{note}\n\n:::\n\nafter"
        );
        let after = before.replacen("::: columns", "::: columns 20/50/30", 1);
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let width = px(700.);
            let build = |text: &str, doc: &DocumentCache, window: &mut Window| {
                let mut rows = RowCache::empty();
                rowcache::rebuild_for_test(
                    &mut rows,
                    text,
                    doc,
                    (0, false),
                    width,
                    (&font, size, style.color),
                    window,
                );
                rows
            };
            let mut doc = DocumentCache::new(&before);
            doc.view.set_column_width(Some(f32::from(width)));
            let mut rows = build(&before, &doc, window);
            let at = before.find("::: columns").unwrap();
            let (new_text, edit) = apply_edit(
                &before,
                at,
                at + "::: columns".len(),
                "::: columns 20/50/30",
            );
            assert_eq!(new_text, after);
            let patch = doc.record_edit(&new_text, edit).unwrap();
            rowcache::patch_for_test(
                &mut rows,
                &new_text,
                &doc,
                patch.touched,
                (0, false),
                (&font, size, style.color),
                window,
            );
            let fresh = build(&new_text, &doc, window);
            let n = new_text.lines().count();
            let counts = |r: &RowCache| {
                (0..n)
                    .map(|i| rowcache::count_at(&r, i))
                    .collect::<Vec<_>>()
            };
            assert_eq!(counts(&rows), counts(&fresh));
        })
        .unwrap();
    }

    /// Text beside a float is as wide as the channel; the paragraph crossing the
    /// float's bottom is cut into the rows beside it and the rest below, at the
    /// whole width, and the two pieces tile the raw line.
    #[gpui_kit::test]
    fn text_flows_around_a_float_and_widens_below_it(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;

        let para = "flowing words ".repeat(40);
        let text = format!(
            "::: aside right 40%\n\nfloat\n\nfloat two\n\nfloat three\n\n---\n\n{para}\n\n:::\n"
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let line_height = window.line_height();
            let mut doc = DocumentCache::new(&text);
            doc.view.set_column_width(Some(800.));
            let (region, split) = doc.side_by_side(0).expect("an aside lays out");
            let shaped = shape_region(
                &text,
                &doc,
                (&region, &split),
                (0, false),
                (&font, size, line_height),
                (style.color.into(), None),
                window,
            );
            let flow = shaped
                .columns
                .iter()
                .find(|c| {
                    c.lines.iter().any(|sl| {
                        sl.raw.end > 0
                            && text[doc.line_start_byte(sl.line)..].starts_with("flowing")
                    })
                })
                .expect("a column holds the paragraph");
            let pieces: Vec<_> = flow.lines.iter().collect();
            assert_eq!(pieces.len(), 2, "the paragraph is cut once");
            let (beside, below) = (pieces[0], pieces[1]);
            assert!(beside.width < below.width, "narrower beside the float");
            assert_eq!(beside.x, px(split.x(1)));
            assert_eq!(below.x, px(0.));
            assert_eq!(below.width, px(split.total));
            assert_eq!(beside.raw.end, below.raw.start, "the pieces meet");
            assert_eq!(below.raw.end, para.len());
            assert_eq!(beside.line, below.line);
            assert!(below.y >= beside.y + beside.height);
            // The float itself is on the right at its width.
            let float = shaped.columns.iter().find(|c| c.x > flow.x).unwrap();
            assert_eq!(float.width, px(split.width(0)));
        })
        .unwrap();
    }

    /// A table column moved inside a column of a region: the rows after the
    /// edit's patch must equal a cold rebuild (the list, table and callout
    /// shape of `Columns Test.md` section 5).
    #[gpui_kit::test]
    fn moving_a_table_column_in_a_region_relays_out(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::RowCache;
        use gpui::Window;

        let list = "- a list\n- with three\n  - and a nested\n- items";
        let table = |a: &str, b: &str| {
            format!(
                "| {a} | {b} |Bobs |Foobar | |\n| ---- | ----- |---|---|---|\n| b    | 2     | | | |\n| a    | 1     | | | |"
            )
        };
        let note = "> [!note] A callout\n> inside a column";
        let mk = |t: String| {
            format!(
                "intro\n\n::: columns 30/43/27\n\n{list}\n\n---\n\n{t}\n\n---\n\n{note}\n\n:::\n\nafter"
            )
        };
        let (before, after) = (mk(table("Name", "Value")), mk(table("Value", "Name")));
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let font = style.font();
            let size = style.font_size.to_pixels(window.rem_size());
            let width = px(900.);
            let build = |text: &str, doc: &DocumentCache, window: &mut Window| {
                let mut rows = RowCache::empty();
                rowcache::rebuild_for_test(
                    &mut rows,
                    text,
                    doc,
                    (0, false),
                    width,
                    (&font, size, style.color),
                    window,
                );
                rows
            };
            let mut doc = DocumentCache::new(&before);
            doc.view.set_column_width(Some(f32::from(width)));
            let mut rows = build(&before, &doc, window);
            let at = before.find("| Name | Value |").unwrap();
            let (new_text, edit) = apply_edit(
                &before,
                at,
                at + "| Name | Value |".len(),
                "| Value | Name |",
            );
            assert_eq!(new_text, after);
            let patch = doc.record_edit(&new_text, edit).unwrap();
            rowcache::patch_for_test(
                &mut rows,
                &new_text,
                &doc,
                patch.touched,
                (0, false),
                (&font, size, style.color),
                window,
            );
            let fresh = build(&new_text, &doc, window);
            let n = new_text.lines().count();
            let counts = |r: &RowCache| {
                (0..n)
                    .map(|i| rowcache::count_at(&r, i))
                    .collect::<Vec<_>>()
            };
            assert_eq!(counts(&rows), counts(&fresh));
        })
        .unwrap();
    }

    /// A column holding a table cannot be dragged narrower than the table's
    /// minimum box, so the box never paints into the next column.
    #[test]
    fn a_columns_minimum_is_its_widest_tables_minimum_box() {
        use super::super::doc::DocumentCache;
        let text = "::: columns\n\ntext\n\n---\n\n| a | b | c | d | e |\n| - | - | - | - | - |\n| 1 | 2 | 3 | 4 | 5 |\n\n:::\n";
        let doc = DocumentCache::new(text);
        let region = doc.columns()[0];
        assert_eq!(super::min_column_px(&doc, &region, 0, 10.), 0.);
        // Five columns of 3 characters plus 3 of padding each.
        assert_eq!(super::min_column_px(&doc, &region, 1, 10.), 300.);
    }

    /// A divider drag previews its shares in the layout (so the columns' content
    /// follows the pointer) without touching the source, and ends cleanly.
    #[gpui_kit::test]
    fn a_dragged_divider_previews_the_shares_until_it_ends(cx: &mut TestAppContext) {
        use super::super::columns::handles::ColumnDrag;
        let text = "intro\n\n::: columns\n\nleft\n\n---\n\nright\n\n:::\n\nafter";
        let (window, editor) = open_editor(cx, text);
        let shares = |cx: &mut TestAppContext| {
            editor.read_with(cx, |e, _| {
                let mut doc = e.document_cache.borrow_mut();
                doc.view.set_column_width(Some(800.));
                doc.side_by_side(2)
                    .map(|(r, _)| r.percent[..r.count].to_vec())
            })
        };
        assert_eq!(shares(cx), Some(vec![50, 50]));
        editor.update(cx, |e, _| {
            e.column_drag = Some(ColumnDrag {
                region: 0,
                divider: 0,
                percent: vec![70, 30],
            });
        });
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(shares(cx), Some(vec![70, 30]), "the drag is laid out live");
        assert!(editor.read_with(cx, |e, _| e.buffer().text().contains("::: columns\n")));
        editor.update(cx, |e, _| e.column_drag = None);
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        assert_eq!(
            shares(cx),
            Some(vec![50, 50]),
            "ending the drag drops the preview"
        );
    }

    /// A divider drag re-measures only its own region. A full rebuild would
    /// re-seed every line outside the viewport with estimates, growing the
    /// document under an unchanged scroll offset (the view jumped).
    #[gpui_kit::test]
    fn a_divider_drag_leaves_the_rows_outside_its_region_alone(cx: &mut TestAppContext) {
        use super::super::columns::handles::ColumnDrag;
        let text = "a **bold** line\n\nb [[link]] line\n\n::: columns\n\nleft\n\n---\n\nright\n\n:::\n\nc **bold** after\n\nd";
        let (window, editor) = open_editor(cx, text);
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let sums = |cx: &mut TestAppContext| {
            editor.read_with(cx, |e, _| {
                let doc = e.document_cache.borrow();
                let region = doc.columns()[0];
                let rows = e.row_height_cache.borrow();
                let (width, verified, _) = rows.trace_state();
                (
                    rows.prefix_sum(region.start_line),
                    rows.prefix_sum(doc.line_count()) - rows.prefix_sum(region.end_line),
                    width.unwrap_or(Pixels::MAX),
                    verified,
                )
            })
        };
        let before = sums(cx);
        assert_ne!(before.2, px(0.), "the first frame built the cache");
        editor.update(cx, |e, _| {
            e.column_drag = Some(ColumnDrag {
                region: 0,
                divider: 0,
                percent: vec![70, 30],
            });
        });
        // Only the sync, no paint: a paint would verify the rows again and hide
        // a cache that had been thrown away.
        cx.update_window(window, |_, window, cx| {
            editor.update(cx, |e, cx| e.sync_column_view(window, cx));
        })
        .unwrap();
        let after = sums(cx);
        assert_eq!(after.0, before.0, "rows above the region are kept");
        assert_eq!(after.1, before.1, "rows below the region are kept");
        assert_eq!(after.2, before.2, "the cache is not marked for a rebuild");
        assert_eq!(after.3, before.3, "the verified rows are kept");
    }

    /// The other half of [`super::RowCache`]'s invalidation story:
    /// moving the cursor with *no* accompanying edit still needs the old
    /// and new active lines patched (`sync_active_line`), since the active
    /// line always renders raw/unstyled regardless of what the (identical)
    /// underlying text would otherwise classify it as. Compared against
    /// [`exact_row_counts`], not a second `rebuild` call — this is exactly
    /// the test that caught `rebuild`'s own estimate path not being a valid
    /// oracle any more (see `exact_row_counts`'s own doc comment): a line
    /// that's *inactive* right now but was corrected to its exact value by
    /// an earlier `sync_active_line` call (e.g. line 1 here, bold markup
    /// hidden, briefly active in an earlier loop iteration) must stay at
    /// that exact value, not drift back toward a fresh `rebuild`'s
    /// over-estimate for it.
    #[gpui_kit::test]
    fn row_height_cache_sync_active_line_matches_the_exact_row_counts(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::RowCache;

        // A line long enough that hiding/showing its `**bold**` markup
        // while active/inactive actually changes its row count at a narrow
        // wrap width — otherwise active-vs-not could coincidentally never
        // move the needle this test is checking for.
        let long_bold = "x".repeat(40);
        let text = format!("short\n**{long_bold}**\nanother short line\nyet another line here");
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let available_width = px(80.); // narrow enough to force wrapping

            let doc_cache = DocumentCache::new(&text);
            let mut row_cache = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut row_cache,
                &text,
                &doc_cache,
                (0, true),
                available_width,
                (&font, font_size, text_color),
                window,
            );

            let mut previous = 0;
            // Move the cursor onto the bold line (line 1), then off it again
            // onto line 2 — each a pure cursor move, no edit at all.
            for cursor in [
                text.find(&long_bold).unwrap(),
                text.rfind("another").unwrap(),
                0,
            ] {
                rowcache::sync_for_test(
                    &mut row_cache,
                    &text,
                    &doc_cache,
                    (cursor, true),
                    (&font, font_size, text_color),
                    window,
                );

                // A patched line is shaped, so it equals what painting sees (this
                // short note's other lines are estimated right).
                let expected: Vec<usize> = (0..doc_cache.line_count())
                    .map(|i| {
                        rowcache::shaped_count_for_test(
                            &text,
                            &doc_cache,
                            i,
                            (cursor, true),
                            available_width,
                            (&font, font_size, text_color),
                            window,
                        )
                    })
                    .collect();
                // The two lines the caret moved between are shaped; the rest
                // keep the cold count (it can be a row short of the shaped one
                // for a wrapped line: `verify` makes the window exact).
                let cold = exact_row_counts(
                    &text,
                    &doc_cache,
                    (cursor, true),
                    available_width,
                    (&font, font_size, text_color),
                    window,
                );
                let now = doc_cache.line_at(cursor);
                let mut want = cold;
                for line in [previous, now] {
                    want[line] = expected[line] as i64;
                }
                previous = now;
                assert_eq!(row_counts(&row_cache), want, "cursor at {cursor}");
            }
        })
        .unwrap();
    }

    /// Moving the caret within a region re-counts the region's rows from its
    /// shaped layout, not the monospace estimate: the estimate sat in the
    /// verified window until paint, so scrolling to the caret used a region
    /// height far off a float's tall text and the view jumped.
    #[gpui_kit::test]
    fn moving_the_caret_in_a_region_keeps_its_exact_height(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::region::shape_region;
        use super::{RowCache, Window, mono_char_width};

        let para = "flowing words ".repeat(50);
        let text = format!(
            "intro\n\n::: float right 30%\n\nfloat\n\n:::\n\n{para}\n\n{para}\n\n{para}\n\nafter"
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let lh = window.line_height();
            let available_width = px(1100.);

            let mut doc = DocumentCache::new(&text);
            doc.view.set_column_width(Some(1100.));
            let mut rows = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut rows,
                &text,
                &doc,
                (0, false),
                available_width,
                (&font, font_size, text_color),
                window,
            );
            let region = doc.columns()[0];
            let exact = |cursor: usize, window: &mut Window| {
                let (region, split) = doc.side_by_side(region.start_line).unwrap();
                shape_region(
                    &text,
                    &doc,
                    (&region, &split),
                    (cursor, true),
                    (&font, font_size, lh),
                    (text_color.into(), None),
                    window,
                )
                .rows
            };
            for cursor in [
                text.find("flowing").unwrap() + 5,
                text.rfind("flowing").unwrap(),
            ] {
                rowcache::sync_for_test(
                    &mut rows,
                    &text,
                    &doc,
                    (cursor, true),
                    (&font, font_size, text_color),
                    window,
                );
                let cached = (rows.prefix_sum(region.start_line + 1)
                    - rows.prefix_sum(region.start_line)) as usize;
                assert_eq!(cached, exact(cursor, window), "caret at {cursor}");
            }
        })
        .unwrap();
    }

    /// Proves the estimate-then-correct scheme (`RowCache::rebuild`'s
    /// cheap seed, upgraded to the truth line-by-line as `paint_lines`
    /// actually corrects each one — see `RowCache::correct`'s own
    /// doc comment) converges to exactly the same state the old always-
    /// exact path guaranteed, once *every* line has been "viewed": builds
    /// a cold `rebuild` (cheap estimates for every non-table line), then
    /// simulates scrolling through the *entire* 320K-line stress fixture
    /// by calling `correct` for every line with its real computed value
    /// (what `paint_lines` would do for whichever lines are actually
    /// visible, just exhaustively here rather than only for a viewport),
    /// and asserts the fully-corrected result matches [`exact_row_counts`]
    /// everywhere — not just for the handful of lines a real viewport
    /// would reach.
    #[gpui_kit::test]
    fn row_height_cache_converges_to_the_exact_row_counts_once_every_line_has_been_corrected(
        cx: &mut TestAppContext,
    ) {
        use super::super::doc::DocumentCache;
        use super::RowCache;

        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(text) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let available_width = px(800.);

            let doc_cache = DocumentCache::new(&text);
            let mut row_cache = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut row_cache,
                &text,
                &doc_cache,
                (0, false),
                available_width,
                (&font, font_size, text_color),
                window,
            );

            for i in 0..doc_cache.line_count() {
                let real = rowcache::exact_row_count(
                    &Inputs {
                        text: &text,
                        cursor: 0,
                        focused: false,
                        width: available_width,
                        font: &font,
                        font_size,
                        text_color,
                    },
                    &doc_cache,
                    i,
                    window,
                );
                rowcache::correct_for_test(&mut row_cache, i, real);
            }

            let expected = exact_row_counts(
                &text,
                &doc_cache,
                (0, false),
                available_width,
                (&font, font_size, text_color),
                window,
            );
            assert_eq!(row_counts(&row_cache), expected);
        })
        .unwrap();
    }

    /// An image embed alone on an ordinary line is a block of fixed height; the
    /// same text anywhere else (remote, code, quote, list) stays one row, and the
    /// line under the cursor shows its source. The cold rebuild's estimate must
    /// agree with the exact count, or callout bands would sit at stale heights.
    #[gpui_kit::test]
    fn an_image_embed_line_is_a_block_with_the_placeholder_height(cx: &mut TestAppContext) {
        use super::super::doc::DocumentCache;
        use super::super::image::PLACEHOLDER_ROWS;
        use super::RowCache;

        let text = "intro\n![a](a.png)\n![[b.png|200]]\n![x](https://e.com/x.png)\n```\n![c](c.png)\n```\n\
                    > ![d](d.png)\n- ![e](e.png)\nend";
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let width = px(800.);
            let doc = DocumentCache::new(text);
            let mut rows = RowCache::empty();
            rowcache::rebuild_for_test(
                &mut rows,
                text,
                &doc,
                (0, false),
                width,
                (&font, font_size, text_color),
                window,
            );
            let exact = exact_row_counts(
                text,
                &doc,
                (0, false),
                width,
                (&font, font_size, text_color),
                window,
            );
            let block = PLACEHOLDER_ROWS as i64;
            assert_eq!(exact, [1, block, block, 1, 1, 1, 1, 1, 1, 1]);
            assert_eq!(row_counts(&rows), exact, "the estimate must match");
            // The cursor on the embed shows its source.
            let on_embed = doc.line_start_byte(1);
            let active = rowcache::exact_row_count(
                &Inputs {
                    text,
                    cursor: on_embed,
                    focused: true,
                    width,
                    font: &font,
                    font_size,
                    text_color,
                },
                &doc,
                1,
                window,
            );
            assert_eq!(active, 1);
        })
        .unwrap();
    }

    /// Once an embed's header has been read off-thread, its block is as tall as
    /// the image is drawn (here 100px wide of a 400x200 file, so 50px), and the
    /// rows are rebuilt for the whole document without any paint of that line.
    #[gpui_kit::test]
    fn an_image_block_takes_its_real_height_once_the_header_is_read(cx: &mut TestAppContext) {
        use super::super::image::PLACEHOLDER_ROWS;
        let dir = tempfile::tempdir().unwrap();
        // PNG signature plus an IHDR chunk: all a header read looks at.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&400u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(dir.path().join("p.png"), png).unwrap();

        let (window, editor) = open_editor(cx, "top\n![[p.png|100]]\nend");
        let root = dir.path().to_path_buf();
        editor.update(cx, |editor, cx| {
            editor.set_image_context(root, "note".into(), cx)
        });
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let before = editor.read_with(cx, |e, _| row_counts(&e.row_height_cache.borrow()));
        assert_eq!(before[1], PLACEHOLDER_ROWS as i64, "no header read yet");

        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let line_height = cx
            .update_window(window, |_, window, _| window.line_height())
            .unwrap();
        let expected = ((50. + 4.) / f32::from(line_height)).ceil() as i64;
        let after = editor.read_with(cx, |e, _| row_counts(&e.row_height_cache.borrow()));
        assert_eq!(after, [1, expected, 1]);
    }

    /// A visible block's decode runs off-thread at the drawn width (not the
    /// file's), lands in the shared cache, and is drawn without a window
    /// error; a block that cannot be decoded keeps its placeholder.
    #[gpui_kit::test]
    fn a_visible_image_is_decoded_at_its_drawn_width(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(400, 200, image::Rgba([200, 30, 30, 255]))
            .save(dir.path().join("p.png"))
            .unwrap();
        std::fs::write(dir.path().join("bad.png"), b"not an image").unwrap();

        let (window, editor) = open_editor(cx, "top\n![[p.png|100]]\n![[bad.png]]\nend");
        let root = dir.path().to_path_buf();
        editor.update(cx, |editor, cx| {
            editor.set_image_context(root, "note".into(), cx)
        });
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let path: std::sync::Arc<std::path::Path> =
            dir.path().canonicalize().unwrap().join("p.png").into();
        let (ready, bytes) = cx.update(|cx| {
            let cache = cx.default_global::<super::super::image::Cache>();
            (cache.is_ready(&path), cache.bytes())
        });
        assert!(ready, "the visible image was decoded");
        // 100 logical px wide at the test window's scale, 2:1, 4 bytes a pixel.
        let scale = cx
            .update_window(window, |_, window, _| window.scale_factor())
            .unwrap();
        let width = (100. * scale).ceil() as usize;
        assert_eq!(bytes, width * (width / 2) * 4);
    }

    /// A real left click on a task's checkbox glyph flips `[ ]`/`[x]` in the
    /// buffer, leaves the cursor where it was, and undoes in one step; a click
    /// on the line's text does not toggle.
    #[gpui_kit::test]
    fn clicking_a_task_checkbox_toggles_it_and_keeps_the_cursor(cx: &mut TestAppContext) {
        use gpui_kit::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
        let (window, editor) = open_editor(cx, "intro\n- [ ] todo\n1. [x] done\nend");
        let click = |cx: &mut TestAppContext, dx: f32, line: usize| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("editor").bounds();
                let y = bounds.origin.y + window.line_height() * (line as f32 + 0.5);
                let position = point(bounds.origin.x + px(dx), y);
                let (button, modifiers) = (MouseButton::Left, Default::default());
                let down = MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                };
                window.dispatch_event(PlatformInput::MouseDown(down), cx);
                let up = MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                };
                window.dispatch_event(PlatformInput::MouseUp(up), cx);
            })
            .unwrap();
        };
        let text =
            |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().text().to_string());
        let cursor = |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().cursor());
        let before = cursor(cx);

        click(cx, 12., 1); // the glyph of "- [ ] todo"
        assert_eq!(text(cx), "intro\n- [x] todo\n1. [x] done\nend");
        assert_eq!(cursor(cx), before, "the cursor does not move");
        click(cx, 12., 1);
        assert_eq!(
            text(cx),
            "intro\n- [ ] todo\n1. [x] done\nend",
            "toggles back"
        );
        click(cx, 45., 2); // an ordered item keeps its number before the glyph
        assert_eq!(text(cx), "intro\n- [ ] todo\n1. [ ] done\nend");
        click(cx, 45., 2);
        click(cx, 400., 1);
        assert_eq!(
            text(cx),
            "intro\n- [ ] todo\n1. [x] done\nend",
            "text is not the box"
        );
        editor.update(cx, |e, _| e.buffer_mut().undo());
        assert_eq!(
            text(cx),
            "intro\n- [ ] todo\n1. [ ] done\nend",
            "one undo step each"
        );
    }

    /// A `[!note]-` callout starts folded: its body has no rows. A click on its
    /// rendered header opens it and a second folds it again, without touching
    /// the text; typing in the open body keeps it open; a cursor placed in a
    /// folded body opens it. A callout with no marker is not foldable.
    #[gpui_kit::test]
    fn a_callout_header_click_folds_its_body(cx: &mut TestAppContext) {
        use gpui_kit::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
        let source = "intro\n> [!note]- Title\n> one\n> two\n\nend\n\n> [!tip] Plain\n> body";
        let (window, editor) = open_editor(cx, source);
        let rows = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
            editor.read_with(cx, |e, _| row_counts(&e.row_height_cache.borrow()))
        };
        let click_line = |cx: &mut TestAppContext, line: usize| {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("editor").bounds();
                let y = bounds.origin.y + window.line_height() * (line as f32 + 0.5);
                let position = point(bounds.origin.x + px(60.), y);
                let (button, modifiers) = (MouseButton::Left, Default::default());
                let down = MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                };
                window.dispatch_event(PlatformInput::MouseDown(down), cx);
                let up = MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                };
                window.dispatch_event(PlatformInput::MouseUp(up), cx);
            })
            .unwrap();
        };
        assert_eq!(
            rows(cx),
            [1, 1, 0, 0, 1, 1, 1, 1, 1],
            "the `-` starts folded"
        );

        click_line(cx, 1);
        assert_eq!(rows(cx), [1, 1, 1, 1, 1, 1, 1, 1, 1], "opened");
        let text = editor.read_with(cx, |e, _| e.buffer().text().to_string());
        assert_eq!(text, source, "folding never edits the note");

        editor.update(cx, |e, cx| {
            e.buffer_mut()
                .set_cursor(source.find("one").unwrap(), false);
            cx.notify();
        });
        click_line(cx, 1);
        assert_eq!(rows(cx), [1, 1, 0, 0, 1, 1, 1, 1, 1], "folded again");
        let cursor = editor.read_with(cx, |e, _| e.buffer().cursor());
        assert_eq!(
            cursor,
            source.find("> [!note]").unwrap(),
            "the cursor left the body"
        );

        // A cursor moved into the body opens it.
        editor.update(cx, |e, cx| {
            e.buffer_mut()
                .set_cursor(source.find("two").unwrap(), false);
            cx.notify();
        });
        assert_eq!(rows(cx)[2..4], [1, 1]);

        // A callout without a marker has nothing to fold: the click moves the cursor.
        click_line(cx, 7);
        assert_eq!(rows(cx), [1, 1, 1, 1, 1, 1, 1, 1, 1]);
    }

    /// Table cell editing, driven through the real key and text paths.
    mod table_cells {
        use super::super::{Grid, Handle, TableColumns, render_params, table};
        use super::*;
        const TABLE: &str = "| a | b |\n|---|---|\n| c | d |\n\nend";

        /// An editor focused with its caret at byte `at`, one frame rendered.
        fn open_at(
            cx: &mut TestAppContext,
            text: &'static str,
            at: usize,
        ) -> (AnyWindowHandle, Entity<Editor>) {
            let (window, editor) = open_editor(cx, text);
            cx.update_window(window, |_, window, cx| {
                focus_editor(window, cx);
                editor.update(cx, |e, _| e.buffer_mut().set_cursor(at, false));
                window.render_frame(cx);
            })
            .unwrap();
            (window, editor)
        }

        fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
            cx.update_window(window, |_, window, cx| window.press(key, cx))
                .unwrap();
        }

        fn type_text(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
            cx.update_window(window, |_, window, cx| {
                window.input(text, cx);
                window.render_frame(cx);
            })
            .unwrap();
        }

        fn state(cx: &mut TestAppContext, editor: &Entity<Editor>) -> (String, usize) {
            editor.read_with(cx, |e, _| {
                (e.buffer().text().to_string(), e.buffer().cursor())
            })
        }

        /// What the row cache holds must be what a cold rebuild would give.
        fn assert_rows_are_exact(
            cx: &mut TestAppContext,
            window: AnyWindowHandle,
            editor: &Entity<Editor>,
        ) {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let cached: Vec<i64> = editor.read_with(cx, |e, _| {
                    let rows = e.row_height_cache.borrow();
                    (0..rows.line_count())
                        .map(|l| rows.prefix_sum(l + 1) - rows.prefix_sum(l))
                        .collect()
                });
                editor.update(cx, |e, _| {
                    e.row_height_cache
                        .borrow_mut()
                        .invalidate(Invalidate::Width)
                });
                window.render_frame(cx);
                let fresh: Vec<i64> = editor.read_with(cx, |e, _| {
                    let rows = e.row_height_cache.borrow();
                    (0..rows.line_count())
                        .map(|l| rows.prefix_sum(l + 1) - rows.prefix_sum(l))
                        .collect()
                });
                assert_eq!(cached, fresh);
            })
            .unwrap();
        }

        #[gpui_kit::test]
        fn typing_edits_the_cell_in_place_and_escapes_pipes(cx: &mut TestAppContext) {
            let at = TABLE.find('c').unwrap() + 1;
            let (window, editor) = open_at(cx, TABLE, at);
            type_text(cx, window, "x|y");
            let (text, _) = state(cx, &editor);
            assert_eq!(text, TABLE.replacen("| c |", "| cx\\|y |", 1));
            assert_rows_are_exact(cx, window, &editor);
        }

        #[gpui_kit::test]
        fn spaces_typed_at_the_end_of_the_last_cell_keep_the_row_height(cx: &mut TestAppContext) {
            let text = "| a | b |\n|---|---|\n| c | de |";
            let (window, editor) = open_at(cx, text, text.len() - 2);
            let rows = |cx: &mut TestAppContext| {
                editor.read_with(cx, |e, _| {
                    let r = e.row_height_cache.borrow();
                    r.prefix_sum(r.line_count())
                })
            };
            let before = rows(cx);
            for _ in 0..3 {
                type_text(cx, window, " ");
                assert_eq!(rows(cx), before);
            }
            assert_rows_are_exact(cx, window, &editor);
        }

        #[gpui_kit::test]
        fn backspace_and_delete_stop_at_the_cells_border(cx: &mut TestAppContext) {
            let at = TABLE.find('c').unwrap();
            let (window, editor) = open_at(cx, TABLE, at);
            press(cx, window, "backspace");
            assert_eq!(state(cx, &editor), (TABLE.to_string(), at));
            press(cx, window, "right");
            press(cx, window, "delete");
            assert_eq!(state(cx, &editor).0, TABLE, "the pipe after the cell stays");
        }

        #[gpui_kit::test]
        fn backspace_removes_an_escaped_pipe_whole(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('c').unwrap() + 1);
            type_text(cx, window, "|");
            press(cx, window, "backspace");
            assert_eq!(state(cx, &editor).0, TABLE);
        }

        #[gpui_kit::test]
        fn tab_and_shift_tab_move_between_cells_across_rows(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('a').unwrap());
            let cell = |cx: &mut TestAppContext| {
                let (text, cursor) = state(cx, &editor);
                let _ = cursor;
                let start = editor.read_with(cx, |e, _| e.buffer().selection().map(|s| s.start));
                text[start.unwrap_or(cursor)..].chars().next().unwrap()
            };
            press(cx, window, "tab");
            assert_eq!(cell(cx), 'b');
            press(cx, window, "tab");
            assert_eq!(cell(cx), 'c', "wraps to the next row, past the delimiter");
            press(cx, window, "shift-tab");
            assert_eq!(cell(cx), 'b');
        }

        #[gpui_kit::test]
        fn tab_in_the_last_cell_adds_a_row(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('d').unwrap());
            press(cx, window, "tab");
            let (text, cursor) = state(cx, &editor);
            assert_eq!(text, TABLE.replacen("| c | d |", "| c | d |\n| | |", 1));
            assert_eq!(cursor, TABLE.find("\n\nend").unwrap() + 2);
            assert_rows_are_exact(cx, window, &editor);
        }

        #[gpui_kit::test]
        fn arrows_cross_cell_borders_but_never_the_pipes(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('a').unwrap() + 1);
            press(cx, window, "right");
            assert_eq!(state(cx, &editor).1, TABLE.find('b').unwrap());
            press(cx, window, "left");
            assert_eq!(state(cx, &editor).1, TABLE.find('a').unwrap() + 1);
            press(cx, window, "shift-right");
            assert_eq!(
                state(cx, &editor).1,
                TABLE.find('a').unwrap() + 1,
                "a selection does not leave the cell"
            );
        }

        #[gpui_kit::test]
        fn up_and_down_keep_the_column_and_stop_at_the_table_edge(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('d').unwrap());
            press(cx, window, "up");
            assert_eq!(
                state(cx, &editor).1,
                TABLE.find('b').unwrap(),
                "past the delimiter"
            );
            press(cx, window, "down");
            assert_eq!(state(cx, &editor).1, TABLE.find('d').unwrap());
            press(cx, window, "down");
            assert_eq!(
                state(cx, &editor).1,
                TABLE.find('d').unwrap() + 1,
                "stays at the end"
            );
        }

        #[gpui_kit::test]
        fn enter_goes_down_adds_a_row_and_an_empty_row_leaves(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('a').unwrap());
            press(cx, window, "enter");
            assert_eq!(state(cx, &editor).1, TABLE.find('c').unwrap());
            press(cx, window, "enter");
            let added = TABLE.replacen("| c | d |", "| c | d |\n| | |", 1);
            let (text, cursor) = state(cx, &editor);
            assert_eq!(text, added);
            assert_eq!(cursor, added.find("| |").unwrap() + 1);
            press(cx, window, "enter");
            let (text, cursor) = state(cx, &editor);
            assert_eq!(text, TABLE, "the empty row is gone");
            assert_eq!(
                cursor,
                TABLE.find("\n\nend").unwrap() + 1,
                "on the blank line after"
            );
        }

        #[gpui_kit::test]
        fn escape_leaves_the_table_and_adds_a_line_at_the_end_of_the_note(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('a').unwrap());
            press(cx, window, "escape");
            assert_eq!(state(cx, &editor).1, TABLE.find("\n\nend").unwrap() + 1);
            let ends = "| a |\n|---|\n| b |";
            let (window, editor) = open_at(cx, ends, ends.find('b').unwrap());
            press(cx, window, "escape");
            assert_eq!(state(cx, &editor), (format!("{ends}\n"), ends.len() + 1));
        }

        #[gpui_kit::test]
        fn shift_enter_inserts_a_line_break_and_backspace_removes_it_whole(
            cx: &mut TestAppContext,
        ) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('c').unwrap() + 1);
            press(cx, window, "shift-enter");
            assert_eq!(
                state(cx, &editor).0,
                TABLE.replacen("| c |", "| c<br> |", 1)
            );
            assert_rows_are_exact(cx, window, &editor);
            // The caret is on the new line, and arrows step over the hidden tag.
            let at = TABLE.find('c').unwrap() + 5;
            assert_eq!(state(cx, &editor).1, at);
            press(cx, window, "left");
            assert_eq!(state(cx, &editor).1, at - 4);
            press(cx, window, "right");
            assert_eq!(state(cx, &editor).1, at);
            press(cx, window, "backspace");
            assert_eq!(state(cx, &editor).0, TABLE);
        }

        #[gpui_kit::test]
        fn ctrl_slash_shows_the_whole_source_and_back(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('c').unwrap());
            press(cx, window, "ctrl-/");
            assert_eq!(
                editor.read_with(cx, |e, _| e.document_cache.borrow().view.raw_table()),
                Some(0)
            );
            assert_rows_are_exact(cx, window, &editor);
            press(cx, window, "ctrl-/");
            assert_eq!(
                editor.read_with(cx, |e, _| e.document_cache.borrow().view.raw_table()),
                None
            );
        }

        #[gpui_kit::test]
        fn the_delimiter_row_is_raw_and_leaving_it_by_arrow_edits_cells(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find("---").unwrap());
            assert_eq!(
                editor.read_with(cx, |e, _| e.document_cache.borrow().view.raw_table()),
                Some(0)
            );
            press(cx, window, "ctrl-/");
            assert_eq!(
                editor.read_with(cx, |e, _| e.document_cache.borrow().view.raw_table()),
                None
            );
            assert_eq!(
                state(cx, &editor).1,
                TABLE.find('c').unwrap(),
                "the caret left the row"
            );
        }

        #[gpui_kit::test]
        fn moving_off_the_delimiter_row_goes_back_to_cell_editing(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find("---").unwrap());
            let raw = |cx: &mut TestAppContext| {
                editor.read_with(cx, |e, _| e.document_cache.borrow().view.raw_table())
            };
            assert_eq!(raw(cx), Some(0));
            press(cx, window, "down");
            assert_eq!(raw(cx), None);
            type_text(cx, window, "|");
            assert_eq!(state(cx, &editor).0, TABLE.replacen("| c |", "| \\|c |", 1));
        }

        #[gpui_kit::test]
        fn shortcuts_edit_rows_and_columns_and_each_is_one_undo(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('c').unwrap());
            press(cx, window, "alt-shift-down");
            let with_row = TABLE.replacen("| c | d |", "| c | d |\n| | |", 1);
            assert_eq!(state(cx, &editor).0, with_row);
            assert_eq!(
                state(cx, &editor).1,
                with_row.find("| |").unwrap() + 1,
                "in the new row"
            );
            assert_rows_are_exact(cx, window, &editor);
            press(cx, window, "alt-shift-right");
            const WITH_COLUMN: &str = "| a | | b |\n|---|---|---|\n| c | | d |\n| | | |\n\nend";
            assert_eq!(state(cx, &editor).0, WITH_COLUMN);
            assert_rows_are_exact(cx, window, &editor);
            press(cx, window, "alt-shift-delete");
            assert_eq!(state(cx, &editor).0, with_row);
            press(cx, window, "alt-shift-backspace");
            assert_eq!(state(cx, &editor).0, TABLE);
            press(cx, window, "ctrl-z");
            assert_eq!(state(cx, &editor).0, with_row, "a delete is one undo");
            press(cx, window, "ctrl-z");
            assert_eq!(state(cx, &editor).0, WITH_COLUMN, "one undo for the column");
        }

        #[gpui_kit::test]
        fn alignment_shortcut_rewrites_the_delimiter_and_row_above_is_off_in_the_header(
            cx: &mut TestAppContext,
        ) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('b').unwrap());
            press(cx, window, "alt-shift-up");
            assert_eq!(state(cx, &editor).0, TABLE, "no row above the header");
            press(cx, window, "alt-shift-c");
            assert_eq!(
                state(cx, &editor).0,
                TABLE.replacen("|---|---|", "|---|:---:|", 1)
            );
            assert_rows_are_exact(cx, window, &editor);
        }

        #[gpui_kit::test]
        fn table_chords_outside_a_table_are_left_to_the_editor(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.len() - 2);
            press(cx, window, "alt-shift-left");
            let selection = editor.read_with(cx, |e, _| e.buffer().selection());
            assert_eq!(selection, Some(TABLE.len() - 3..TABLE.len() - 2));
            assert_eq!(state(cx, &editor).0, TABLE);
        }

        #[gpui_kit::test]
        fn right_click_in_a_table_moves_the_caret_and_has_a_menu(cx: &mut TestAppContext) {
            use gpui_kit::{MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
            let (window, editor) = open_at(cx, TABLE, TABLE.len());
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("editor").bounds();
                // The body row is the third line, below the (one-row) header.
                let position = point(
                    bounds.origin.x + px(20.),
                    bounds.origin.y + window.line_height() * 2.5,
                );
                let (button, modifiers) = (MouseButton::Right, Default::default());
                let down = MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                };
                window.dispatch_event(PlatformInput::MouseDown(down), cx);
                let up = MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                };
                window.dispatch_event(PlatformInput::MouseUp(up), cx);
                window.render_frame(cx);
            })
            .unwrap();
            let line_start = TABLE.find("| c").unwrap();
            let cursor = state(cx, &editor).1;
            assert!(
                (line_start..line_start + 9).contains(&cursor),
                "caret in the clicked row: {cursor}"
            );
            assert!(editor.read_with(cx, |e, _| e.table_menu_state().is_some()));
        }

        const BIG: &str = "| a | b |\n|---|---|\n| c | d |\n| e | f |\n| g | h |\n\nend";

        /// Window position of the table starting the note, and its grid, worked
        /// out the way the editor does (every row here is one line tall).
        fn grid_of(
            cx: &mut TestAppContext,
            window: AnyWindowHandle,
            editor: &Entity<Editor>,
        ) -> (Point<Pixels>, Grid) {
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("editor").bounds();
                let params = editor.update(cx, |e, cx| render_params(e, window, cx));
                let table = editor.read(cx).document_cache.borrow().tables()[0].clone();
                let columns = TableColumns::of(
                    &table,
                    params.available_width,
                    (&params.font, params.font_size, params.text_color),
                    window,
                );
                let line = f32::from(window.line_height());
                let col_x = (0..=columns.widths.len())
                    .map(|c| f32::from(columns.x(table::column_left(&columns.widths, c))))
                    .collect();
                // The header and the delimiter band are 2 lines, then 1 per row.
                let row_y = [0.0, 2.0, 3.0, 4.0, 5.0].map(|n| n * line).to_vec();
                (bounds.origin, Grid { col_x, row_y })
            })
            .unwrap()
        }

        fn mouse(cx: &mut TestAppContext, window: AnyWindowHandle, kind: &str, at: Point<Pixels>) {
            use gpui_kit::{
                MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
            };
            cx.update_window(window, |_, window, cx| {
                let (button, modifiers) = (MouseButton::Left, Default::default());
                let event = match kind {
                    "down" => PlatformInput::MouseDown(MouseDownEvent {
                        button,
                        position: at,
                        modifiers,
                        click_count: 1,
                        first_mouse: false,
                    }),
                    "up" => PlatformInput::MouseUp(MouseUpEvent {
                        button,
                        position: at,
                        modifiers,
                        click_count: 1,
                    }),
                    _ => PlatformInput::MouseMove(MouseMoveEvent {
                        position: at,
                        pressed_button: (kind == "drag").then_some(button),
                        modifiers,
                    }),
                };
                window.dispatch_event(event, cx);
                window.render_frame(cx);
            })
            .unwrap();
        }

        fn centre_of(grid: &Grid, origin: Point<Pixels>, handle: Handle) -> Point<Pixels> {
            let (x, y, w, h) = table::handles::rect_of(grid, handle);
            origin + point(px(x + w / 2.0), px(y + h / 2.0))
        }

        #[gpui_kit::test]
        fn the_controls_follow_the_pointer_over_a_table_and_leave_with_it(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, BIG, BIG.len());
            let (origin, grid) = grid_of(cx, window, &editor);
            let hover =
                |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.table_hover.clone());
            mouse(
                cx,
                window,
                "move",
                origin + point(px(grid.col_x[1] + 4.0), px(grid.row_y[1] + 3.0)),
            );
            let (table, shown) = hover(cx).expect("over the table");
            assert_eq!(table, 0);
            assert!(
                shown.contains(&Handle::ColumnGrip(0)) || shown.contains(&Handle::ColumnGrip(1))
            );
            assert!(shown.contains(&Handle::RowGrip(1)));
            mouse(
                cx,
                window,
                "move",
                origin + point(px(10.0), px(grid.row_y[4] + 120.0)),
            );
            assert_eq!(hover(cx), None, "gone below the table");
        }

        #[gpui_kit::test]
        fn a_plus_on_the_edge_adds_a_row_or_column_there(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, BIG, BIG.len());
            let (origin, grid) = grid_of(cx, window, &editor);
            // The boundary above the third body row ("g").
            let plus = centre_of(&grid, origin, Handle::AddRow(3));
            mouse(cx, window, "down", plus);
            mouse(cx, window, "up", plus);
            assert_eq!(
                state(cx, &editor).0,
                BIG.replacen("| g | h |", "| | |\n| g | h |", 1).replacen(
                    "| e | f |\n| | |\n| g",
                    "| e | f |\n| | |\n| g",
                    1
                )
            );
            assert_rows_are_exact(cx, window, &editor);
            let (origin, grid) = grid_of(cx, window, &editor);
            let plus = centre_of(&grid, origin, Handle::AddColumn(0));
            mouse(cx, window, "down", plus);
            mouse(cx, window, "up", plus);
            assert!(
                state(cx, &editor)
                    .0
                    .starts_with("| | a | b |\n|---|---|---|")
            );
        }

        #[gpui_kit::test]
        fn dragging_a_row_grip_moves_the_row_as_one_undo(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, BIG, BIG.len());
            let (origin, grid) = grid_of(cx, window, &editor);
            let grip = centre_of(&grid, origin, Handle::RowGrip(1));
            mouse(cx, window, "down", grip);
            let target = origin + point(px(20.0), px(grid.row_y[4] - 2.0));
            mouse(cx, window, "drag", target);
            assert_eq!(
                editor.read_with(cx, |e, _| e.table_drag.map(|d| d.over)),
                Some(4),
                "over the bottom boundary"
            );
            mouse(cx, window, "up", target);
            assert_eq!(
                state(cx, &editor).0,
                "| a | b |\n|---|---|\n| e | f |\n| g | h |\n| c | d |\n\nend"
            );
            assert_eq!(editor.read_with(cx, |e, _| e.table_drag), None);
            assert_rows_are_exact(cx, window, &editor);
            press(cx, window, "ctrl-z");
            assert_eq!(state(cx, &editor).0, BIG);
        }

        #[gpui_kit::test]
        fn dragging_a_column_grip_moves_the_column_with_its_alignment(cx: &mut TestAppContext) {
            let text = "| a | b |\n|:--|--:|\n| c | d |\n\nend";
            let (window, editor) = open_at(cx, text, text.len());
            let (origin, grid) = grid_of(cx, window, &editor);
            let grip = centre_of(&grid, origin, Handle::ColumnGrip(0));
            mouse(cx, window, "down", grip);
            let target = origin + point(px(grid.col_x[2] - 2.0), px(10.0));
            mouse(cx, window, "drag", target);
            mouse(cx, window, "up", target);
            assert_eq!(
                state(cx, &editor).0,
                "| b | a |\n|--:|:--|\n| d | c |\n\nend"
            );
        }

        #[gpui_kit::test]
        fn select_all_selects_the_cell_and_paste_is_escaped(cx: &mut TestAppContext) {
            let (window, editor) = open_at(cx, TABLE, TABLE.find('c').unwrap());
            press(cx, window, "ctrl-a");
            assert_eq!(
                editor.read_with(cx, |e, _| e.buffer().selection()),
                Some(TABLE.find('c').unwrap()..TABLE.find('c').unwrap() + 1)
            );
            cx.update(|cx| {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("p|q\nr".into()))
            });
            press(cx, window, "ctrl-v");
            assert_eq!(
                state(cx, &editor).0,
                TABLE.replacen("| c |", "| p\\|q<br>r |", 1)
            );
        }
    }
}

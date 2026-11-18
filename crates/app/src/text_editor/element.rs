use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::WindowExt;
use gpui_kit::*;

use super::Editor;
use super::block_preview::LineRole;
use super::callout::{Callout, callout_index_at};
use super::colors::{InlineColors, callout_role};
use super::document_cache::{DocumentCache, EditPatch};
use super::fenwick::Fenwick;
use super::link_table::LinkTable;
use super::live_preview::{
    DisplayLine, InlineStyle, LineContext, LineIndent, build_callout_header_line,
    build_display_line, build_display_line_in_context, build_table_row, hidden_line,
    is_thematic_break, line_indent,
};
use super::paint_trace::{FrameInfo, PaintTrace};
use super::table_preview::{self, Table, table_index_at};
use crate::debug_server::targets::DebugName;
use crate::theme::{Alpha, Role, ThemeEngine};

/// Resolves `line_index`'s [`Callout`] (if it belongs to one at all) from
/// the resolved `callouts` — same shape as `table_line_kind`.
pub(super) fn callout_at_line(callouts: &[Callout], line_index: usize) -> Option<&Callout> {
    callout_index_at(callouts, line_index).map(|idx| &callouts[idx])
}

/// Everything besides its text that decides how one line is displayed. Built
/// by [`LineState::of`] from the document's caches wherever a line is shaped
/// or measured (painting, hit-testing, row counts, the IME's caret bounds), so
/// they cannot disagree about what the line looks like.
pub(super) struct LineState<'a> {
    pub role: LineRole,
    pub block_ctx: LineContext,
    pub table_kind: TableLineKind<'a>,
    pub table_active: bool,
    pub callout: Option<&'a Callout>,
    pub callout_is_header: bool,
    /// The cursor is on this line, so its Markdown is shown raw.
    pub active: bool,
    /// Raw byte ranges of links on the line whose target is missing.
    pub broken: &'a [Range<usize>],
}

impl<'a> LineState<'a> {
    pub(super) fn of(doc: &'a DocumentCache, i: usize, cursor: usize, focused: bool) -> Self {
        let line_range = doc.line_start_byte(i)..doc.line_end_byte(i);
        let callout = callout_at_line(doc.callouts(), i);
        LineState {
            role: doc.line_roles()[i],
            block_ctx: doc.block_contexts()[i],
            table_kind: table_line_kind(doc.tables(), i),
            table_active: is_table_active(doc, i, cursor, focused),
            callout,
            callout_is_header: callout.is_some_and(|c| c.is_header(i)),
            active: is_active_line(&line_range, cursor, focused),
            broken: &[],
        }
    }

    /// An ordinary line outside any block, for tests to vary with struct update.
    #[cfg(test)]
    pub(super) fn plain() -> Self {
        LineState {
            role: LineRole::Plain,
            block_ctx: LineContext::Plain,
            table_kind: TableLineKind::None,
            table_active: false,
            callout: None,
            callout_is_header: false,
            active: false,
            broken: &[],
        }
    }
}

/// Which table-related rendering a `LineRole::Plain` line needs, resolved
/// by the caller (which already knows the line's absolute index, needed to
/// tell a table's delimiter row from its header/body rows) — see
/// `table_line_kind`.
pub(super) enum TableLineKind<'a> {
    None,
    Delimiter(&'a Table),
    Row(&'a Table),
}

/// Resolves `line_index`'s [`TableLineKind`] from the resolved `tables`.
pub(super) fn table_line_kind(tables: &[Table], line_index: usize) -> TableLineKind<'_> {
    match table_index_at(tables, line_index) {
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
pub(super) fn is_table_active(
    doc: &DocumentCache,
    line_index: usize,
    cursor: usize,
    focused: bool,
) -> bool {
    focused
        && table_index_at(doc.tables(), line_index).is_some_and(|idx| {
            let range = doc.table_byte_range(&doc.tables()[idx]);
            cursor >= range.start && cursor <= range.end
        })
}

/// Pixel width of a callout's left accent bar — thicker than a plain
/// blockquote's own `QUOTE_BAR_WIDTH`, so a callout reads as a distinct,
/// heavier element even before its background tint is noticed.
const CALLOUT_BAR_WIDTH: Pixels = px(3.);

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
pub(super) fn is_active_line(
    line_range: &std::ops::Range<usize>,
    cursor: usize,
    focused: bool,
) -> bool {
    focused && cursor >= line_range.start && cursor <= line_range.end
}

/// Scrolls `scroll_handle`'s tracked container (`app.rs`'s note-editor-pane
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
fn autoscroll_to_cursor(
    scroll_handle: &ScrollHandle,
    content_bounds: Bounds<Pixels>,
    cursor_top: Pixels,
    cursor_height: Pixels,
) {
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

/// How many lines of overscan (shaped/painted even though just outside the
/// viewport) `visible_line_range` pads the true viewport range by — a
/// smooth-scroll/wrap-estimate-slop margin, not load-bearing for
/// correctness (every line still gets exactly one entry in [`LineLayout`]
/// regardless).
const OVERSCAN_LINES: usize = 20;

/// A persistent, edit-patched cache of every line's *row count* (how many
/// wrapped visual rows a line occupies, per [`wrapped_row_count`]) — the
/// per-paint piece `analyze_line_layout` used to re-derive from scratch for
/// every line in the document, every paint, by calling `build_display_lines`
/// (the real per-line Live Preview dispatch/inline-scan, not just
/// arithmetic) even though `document_cache::DocumentCache` had already made
/// *classification itself* incremental. Confirmed by direct measurement to
/// be the dominant remaining per-keystroke cost on the 320K-line stress
/// fixture (~140-155ms) once classification dropped to sub-1ms — this cache
/// is what closes that gap, mirroring `DocumentCache`'s own "whole-document
/// pass once, patch per edit after that" shape.
///
/// Stores row counts in a [`Fenwick`] tree (not a flat `Vec`) for the same
/// reason `DocumentCache::line_index` does: a point update to one line's row
/// count shifts every *subsequent* line's cumulative offset, which a flat
/// array would need an O(suffix length) walk to keep in sync — the whole
/// cost this cache exists to avoid. `line_height` is deliberately *not*
/// baked into the stored values (unlike the old flat `cumulative: Vec<Pixels>`
/// this replaces) — row counts are a property of wrapping alone, so a
/// `line_height`-only change (a font-size bump that doesn't change character
/// width) wouldn't need to invalidate anything if this cache tracked pixels
/// directly; in practice `font_size` changing also changes `char_width`
/// (this editor's font is always monospace), so the [`Self::available_width`]/
/// [`Self::char_width`] fingerprint below still catches it, but the
/// separation keeps the *reason* for each invalidation trigger distinct.
///
/// Three invalidation triggers, matching exactly what a line's row count
/// can actually depend on:
/// - **An edit** — patched eagerly, at the same call site `DocumentCache`
///   itself is patched (`Editor::record_edit`), using the
///   touched-line range `DocumentCache::record_edit` returns. Needs
///   `window`/font/width at that point, which weren't previously computed
///   outside of paint — see [`RenderParams`].
/// - **The active line changing with no edit** (arrow keys, a mouse click
///   elsewhere) — nothing above fires for this, since cursor-only movement
///   is scattered across many call sites in `editor.rs`/mouse handling, none
///   of which report a `TextEdit`. Deliberately *not* hooked at each of
///   those call sites — instead handled lazily, the same way
///   `last_autoscrolled_cursor` already catches "the cursor moved" without
///   being wired into every mover: [`Self::sync_active_line`] is called from
///   `analyze_line_layout` (so every paint, cheaply) and just compares the
///   current active line against what was cached, patching at most the old
///   and new active line if they differ.
/// - **A rendering-context change** (window resize, font/size change) —
///   caught by the same `analyze_line_layout` call via
///   [`Self::needs_full_rebuild`]'s `available_width`/`char_width`
///   fingerprint comparison; a mismatch forces a full [`Self::rebuild`]
///   (the same O(document) cost `analyze_line_layout` used to pay on
///   *every* paint, now paid only on an actual resize/font change, or the
///   very first paint after opening a note).
pub(super) struct RowHeightCache {
    rows: Fenwick,
    /// Which line (if any) was rendered raw/unstyled — [`is_active_line`] —
    /// the last time this cache was built or patched. Compared against the
    /// current cursor's line on every [`Self::sync_active_line`] call so a
    /// pure cursor move (no edit) patches just the lines whose active
    /// status actually changed.
    active_line: Option<usize>,
    /// The `available_width` [`Self::rebuild`] last ran with — part of the
    /// render-context fingerprint `needs_full_rebuild` checks.
    available_width: Pixels,
    /// This editor's monospace advance width (see [`mono_char_width`]) the
    /// last time [`Self::rebuild`] ran — captures both font family/weight
    /// and font size changes in one comparable value, rather than needing
    /// `Font: PartialEq` (which this type deliberately avoids depending on).
    char_width: Pixels,
    /// Lines (one contiguous window, the last painted) whose row counts are
    /// known exact, not [`estimate_row_count`] over-estimates. Reset by a
    /// rebuild or a line-count change; see [`LineLayout::verify_rows`].
    verified: std::ops::Range<usize>,
    /// Corrections `paint_lines`' own per-line loop still had to make: after
    /// [`LineLayout::verify_rows`] there should be none.
    pub(super) late_corrections: usize,
}

impl RowHeightCache {
    /// Before the first real paint has ever populated this — `available_
    /// width`/`char_width` are left at `0`, a sentinel no real font/width
    /// ever produces, so [`Self::needs_full_rebuild`] is unconditionally
    /// true until the first [`Self::rebuild`] call.
    pub(super) fn empty() -> Self {
        Self {
            rows: Fenwick::new(&[]),
            active_line: None,
            available_width: px(0.),
            char_width: px(0.),
            verified: 0..0,
            late_corrections: 0,
        }
    }

    pub(super) fn line_count(&self) -> usize {
        self.rows.len()
    }

    /// The cumulative row count of lines `0..i` — `i == line_count()` (one
    /// past the last line) is valid too, giving the document's total row
    /// count.
    pub(super) fn prefix_sum(&self, i: usize) -> i64 {
        self.rows.prefix_sum(i)
    }

    /// Whether [`Self::rebuild`] needs to run again before this cache's row
    /// counts can be trusted at all: either the document's line count has
    /// drifted out of sync with `doc` (a line-count-changing edit, or this
    /// cache never having been built at all), or the render context this
    /// cache's existing row counts were computed against has changed.
    pub(super) fn needs_full_rebuild(
        &self,
        doc: &DocumentCache,
        available_width: Pixels,
        char_width: Pixels,
    ) -> bool {
        self.line_count() != doc.line_count()
            || self.available_width != available_width
            || self.char_width != char_width
    }

    /// Full O(document) rebuild — paid only when [`Self::needs_full_
    /// rebuild`] says so (first paint after opening a note, a resize, or a
    /// font/size change). Unlike the very first version of this cache,
    /// this no longer pays `build_display_lines`'s real Live-Preview
    /// inline-scan cost for every line: [`estimate_row_count`] gives a
    /// cheap, allocation-free over-estimate for ordinary lines (confirmed
    /// by direct measurement to cut cold-open allocation count from ~4
    /// million to a small fraction of that on the 320K-line stress
    /// fixture — see HISTORY.md), falling back to the exact computation
    /// only for table rows/delimiters, where an estimate based on raw
    /// source length isn't safely guaranteed to be an over-estimate (a
    /// padded cell can display *wider* than its trimmed source). Any
    /// line seeded with an estimate gets corrected to the exact value the
    /// first time it's actually painted — see `paint_lines`'s own call to
    /// `LineLayout::correct_row_count`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rebuild(
        &mut self,
        text: &str,
        doc: &DocumentCache,
        cursor: usize,
        focused: bool,
        available_width: Pixels,
        char_width: Pixels,
        font: &Font,
        font_size: Pixels,
        text_color: Hsla,
        window: &mut Window,
    ) {
        let line_count = doc.line_count();
        let starts = doc.line_start_offsets();
        let view = DocView {
            roles: doc.line_roles(),
            contexts: doc.block_contexts(),
            tables: doc.tables(),
            callouts: doc.callouts(),
            starts: &starts,
        };
        let rows = RowParams {
            active_table: active_table(doc, cursor, focused),
            available_width,
            char_width,
        };
        let (mut counts, shaped) = pure_row_counts(text, &view, &rows);
        // Lines that need the window (an active or irregular table row) are the
        // few the threads left out.
        for i in shaped {
            let count = row_count_for_line(
                text,
                doc,
                i,
                cursor,
                focused,
                char_width,
                font,
                font_size,
                text_color,
                available_width,
                window,
            );
            counts[i] = count as i64;
        }
        assert_eq!(counts.len(), line_count);
        self.rows = Fenwick::from_vec(counts);
        self.active_line = if focused {
            Some(doc.line_at(cursor))
        } else {
            None
        };
        self.available_width = available_width;
        self.char_width = char_width;
        self.verified = 0..0;
    }

    /// Mirrors `patch`'s line-count change in the row counts, so
    /// [`Self::patch_lines`] can then re-derive just the touched lines.
    /// A no-op when this cache wasn't in sync with the document before the
    /// edit (never built, say): [`Self::needs_full_rebuild`] then sees the
    /// line-count mismatch and forces a full rebuild.
    pub(super) fn resize_for(&mut self, patch: &EditPatch, doc: &DocumentCache) {
        let old_count = doc.line_count() as isize - patch.line_delta;
        if patch.line_delta != 0 && self.line_count() as isize == old_count {
            self.rows.resize(patch.splice_at, patch.line_delta);
            self.verified = 0..0;
        }
    }

    /// Recomputes every line in `touched`, patching the Fenwick tree in
    /// place — the real incremental path for a text edit, called with the
    /// touched-line range `DocumentCache::record_edit` already returned for
    /// this same edit. Assumes [`Self::needs_full_rebuild`] is already
    /// false (checked by the caller) — a render-context change is handled
    /// by a full [`Self::rebuild`] instead, never this.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn patch_lines(
        &mut self,
        text: &str,
        doc: &DocumentCache,
        touched: std::ops::Range<usize>,
        cursor: usize,
        focused: bool,
        char_width: Pixels,
        font: &Font,
        font_size: Pixels,
        text_color: Hsla,
        window: &mut Window,
    ) {
        for i in touched {
            let old = self.rows.prefix_sum(i + 1) - self.rows.prefix_sum(i);
            let new = row_count_for_line(
                text,
                doc,
                i,
                cursor,
                focused,
                char_width,
                font,
                font_size,
                text_color,
                self.available_width,
                window,
            );
            self.rows.add_at(i, new as i64 - old);
        }
        self.active_line = if focused {
            Some(doc.line_at(cursor))
        } else {
            None
        };
    }

    /// Patches whichever of the previously-active and newly-active line (at
    /// most two lines) actually changed status since this cache was last
    /// built or patched — the lazy, render-time-only trigger for "the
    /// cursor moved with no accompanying edit" (see this struct's own doc
    /// comment for why that case isn't hooked at every cursor-moving call
    /// site instead). A no-op (not even a `Fenwick` touch) when the active
    /// line hasn't actually changed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sync_active_line(
        &mut self,
        text: &str,
        doc: &DocumentCache,
        cursor: usize,
        focused: bool,
        char_width: Pixels,
        font: &Font,
        font_size: Pixels,
        text_color: Hsla,
        window: &mut Window,
    ) {
        let new_active = if focused {
            Some(doc.line_at(cursor))
        } else {
            None
        };
        if new_active == self.active_line {
            return;
        }
        for line in [self.active_line, new_active].into_iter().flatten() {
            if line < self.line_count() {
                let old = self.rows.prefix_sum(line + 1) - self.rows.prefix_sum(line);
                let new = row_count_for_line(
                    text,
                    doc,
                    line,
                    cursor,
                    focused,
                    char_width,
                    font,
                    font_size,
                    text_color,
                    self.available_width,
                    window,
                );
                self.rows.add_at(line, new as i64 - old);
            }
        }
        self.active_line = new_active;
    }

    /// Upgrades line `i`'s row count to `real_rows` if it differs from
    /// what's currently cached — the third invalidation trigger, used by
    /// `paint_lines` to correct a cold-rebuild estimate (see
    /// [`Self::rebuild`]) to the truth using a value it already computed
    /// for real painting, at zero extra cost. Idempotent: a no-op once a
    /// line has already been corrected, or for any line `rebuild` already
    /// computed exactly (table rows/delimiters).
    fn row_count(&self, i: usize) -> i64 {
        self.rows.prefix_sum(i + 1) - self.rows.prefix_sum(i)
    }

    pub(super) fn correct(&mut self, i: usize, real_rows: usize) {
        if i >= self.line_count() {
            return;
        }
        let old = self.rows.prefix_sum(i + 1) - self.rows.prefix_sum(i);
        let delta = real_rows as i64 - old;
        if delta != 0 {
            self.rows.add_at(i, delta);
        }
    }
}

/// The parts of a [`DocumentCache`] a row count reads, as plain slices (the
/// cache itself holds `Rc`s, so it cannot cross to worker threads).
struct DocView<'a> {
    roles: &'a [LineRole],
    contexts: &'a [LineContext],
    tables: &'a [Table],
    callouts: &'a [Callout],
    /// Byte offset of each line, plus the text length.
    starts: &'a [usize],
}

/// What a row count depends on besides the document.
struct RowParams {
    /// The table the cursor is in while focused: it renders as raw text.
    active_table: Option<usize>,
    available_width: Pixels,
    char_width: Pixels,
}

/// The index of the table [`is_table_active`] would call active, found once so
/// workers need not ask per line.
fn active_table(doc: &DocumentCache, cursor: usize, focused: bool) -> Option<usize> {
    if !focused {
        return None;
    }
    let tables = doc.tables();
    let idx = tables.partition_point(|t| doc.table_byte_range(t).end < cursor);
    let table = tables.get(idx)?;
    (doc.table_byte_range(table).start <= cursor).then_some(idx)
}

/// Below this many lines a thread per core costs more than it saves.
const PARALLEL_MIN_LINES: usize = 20_000;

/// Every line's row count that needs no `Window`, split over threads on big
/// notes, plus the lines that do need one (their entry is a placeholder `0`).
fn pure_row_counts(text: &str, view: &DocView, rows: &RowParams) -> (Vec<i64>, Vec<usize>) {
    let line_count = view.roles.len();
    let threads = if line_count < PARALLEL_MIN_LINES {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    };
    let step = line_count.div_ceil(threads).max(1);
    let parts: Vec<(Vec<i64>, Vec<usize>)> = if threads == 1 {
        vec![pure_row_counts_in(text, view, rows, 0..line_count)]
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|k| (k * step).min(line_count)..((k + 1) * step).min(line_count))
                .map(|range| scope.spawn(move || pure_row_counts_in(text, view, rows, range)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                .collect()
        })
    };
    let mut counts = Vec::with_capacity(line_count);
    let mut shaped = Vec::new();
    for (part, part_shaped) in parts {
        counts.extend(part);
        shaped.extend(part_shaped);
    }
    (counts, shaped)
}

/// [`pure_row_counts`] for the lines in `range`.
fn pure_row_counts_in(
    text: &str,
    view: &DocView,
    rows: &RowParams,
    range: std::ops::Range<usize>,
) -> (Vec<i64>, Vec<usize>) {
    let RowParams {
        active_table,
        available_width,
        char_width,
    } = *rows;
    let line_count = view.roles.len();
    let mut counts = Vec::with_capacity(range.len());
    let mut shaped = Vec::new();
    // One sequential pass: `tables` are sorted, so table membership comes from
    // a cursor instead of a binary search per line. What is the same for every
    // row of a table (column widths) is worked out once per table.
    let budget = char_budget(available_width, char_width);
    let one_level_fit = fit_with_indent(available_width, char_width, 1);
    let mut next_table = view.tables.partition_point(|t| t.end_line <= range.start);
    let mut table_state: Option<(usize, Option<Vec<usize>>)> = None;
    for i in range {
        let end = if i + 1 < line_count {
            view.starts[i + 1] - 1
        } else {
            view.starts[i + 1]
        };
        let line = &text[view.starts[i]..end];
        while view.tables.get(next_table).is_some_and(|t| t.end_line <= i) {
            next_table += 1;
        }
        let table = view.tables.get(next_table).filter(|t| t.start_line <= i);
        let count = match table {
            None => estimate_row_count(line, view, i, char_width, available_width, one_level_fit),
            Some(table) => {
                let is_header = callout_at_line(view.callouts, i).is_some_and(|c| c.is_header(i));
                let plain = matches!(view.roles[i], LineRole::Plain) && !is_header;
                if table_state.as_ref().is_none_or(|(n, ..)| *n != next_table) {
                    // A table that fits its width has no wrapped cell in any row,
                    // so its widths (and its rows' text) are never looked at.
                    let widths = if table_preview::fits_unshrunk(&table.columns, budget) {
                        None
                    } else {
                        Some(table_preview::resolve_widths(&table.columns, budget))
                    };
                    table_state = Some((next_table, widths));
                }
                match &table_state {
                    // An inactive, plainly rendered table: a delimiter is one hidden
                    // row, a row is as tall as its tallest wrapped cell.
                    Some((n, widths)) if plain && active_table != Some(*n) => match widths {
                        Some(widths) if !table.is_delimiter(i) => {
                            table_preview::row_line_count(line, widths)
                        }
                        _ => 1,
                    },
                    _ => {
                        shaped.push(i);
                        0
                    }
                }
            }
        };
        counts.push(count as i64);
    }
    (counts, shaped)
}

/// A cheap, allocation-free over-estimate of `line`'s (line `i`'s) row count
/// — the cold-`rebuild` fast path for an *ordinary* line; the caller must
/// not pass a table row or delimiter, which get the exact computation
/// instead (see [`RowHeightCache::rebuild`]'s own doc comment for why). Uses the line's
/// *raw* length rather than running `build_display_lines`'s real Live-
/// Preview inline scan at all: always safe (an over-estimate, never
/// fewer rows than the truth) because every hiding/replacement transform
/// this editor's Live Preview does — markup removal, a bullet/checkbox
/// glyph, a callout's icon + capitalized type name — only ever shrinks
/// what's displayed relative to the raw source, never grows it. Also
/// exact, not just a safe over-estimate, for the active line and for
/// fenced code content, both of which always render their raw text
/// verbatim anyway.
///
/// Still needs the indent level right (a wider wrap width than the real
/// one would *under*-estimate, the one way this could go unsafe) — reuses
/// `live_preview::line_indent` (the same cheap prefix-only check `build_
/// display_line` itself uses, not the expensive scan) combined with the
/// line's already-known `LineContext` (from `DocumentCache`, itself
/// already computed — not rederived here) the same way `build_display_
/// line_in_context` resolves it for real, except for fenced code content,
/// whose indent is always forced to `None` regardless (see
/// `build_display_lines`'s own fence-content override).
fn estimate_row_count(
    line: &str,
    doc: &DocView,
    i: usize,
    char_width: Pixels,
    available_width: Pixels,
    one_level_fit: usize,
) -> usize {
    // A line with no context of its own that doesn't start with `>` is
    // indented at most one level (a list marker; blockquotes need a `>`), so
    // if even that narrower width holds its bytes it is a single row — most
    // lines, decided without looking at its indent at all.
    if matches!(doc.contexts[i], LineContext::Plain)
        && !matches!(doc.roles[i], LineRole::FenceContent)
        && line.as_bytes().first() != Some(&b'>')
        && line.len() <= one_level_fit
    {
        return 1;
    }
    let indent_level = if matches!(doc.roles[i], LineRole::FenceContent) {
        0
    } else {
        match doc.contexts[i] {
            LineContext::Plain => line_indent(line).level(),
            LineContext::ListMarkerLine(depth) => depth,
            LineContext::LazyContinuation(indent) => indent.level(),
        }
    };
    let wrap_width = Some((available_width - indent_px(indent_level)).max(px(1.)));
    wrapped_row_count(line, wrap_width, char_width).max(1)
}

/// The row count for line `i` alone — the same per-line computation
/// `analyze_line_layout`'s old whole-document loop ran unconditionally for
/// every line; now only ever called for a line [`RowHeightCache`] actually
/// needs to (re)compute.
#[allow(clippy::too_many_arguments)]
fn row_count_for_line(
    text: &str,
    doc: &DocumentCache,
    i: usize,
    cursor: usize,
    focused: bool,
    char_width: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    available_width: Pixels,
    window: &mut Window,
) -> usize {
    let line_text = &text[doc.line_start_byte(i)..doc.line_end_byte(i)];
    let state = LineState::of(doc, i, cursor, focused);
    let (display_lines, wrap_width) = build_display_lines(
        line_text,
        &state,
        font,
        font_size,
        text_color,
        available_width,
        window,
    );
    display_lines
        .iter()
        .map(|dl| wrapped_row_count(&dl.display_text, wrap_width, char_width))
        .sum::<usize>()
        .max(1)
}

/// The row count painting will see for line `i`: real shaping, so word
/// wrapping is included. [`row_count_for_line`]'s monospace estimate can come
/// out one row short of that, which `verify_rows` must not inherit.
#[allow(clippy::too_many_arguments)]
fn shaped_row_count_for_line(
    text: &str,
    doc: &DocumentCache,
    i: usize,
    cursor: usize,
    focused: bool,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    available_width: Pixels,
    window: &mut Window,
) -> usize {
    let shaped = shape_display_line(
        &text[doc.line_start_byte(i)..doc.line_end_byte(i)],
        &LineState::of(doc, i, cursor, focused),
        font,
        font_size,
        text_color,
        available_width,
        window,
    );
    let height: Pixels = shaped
        .iter()
        .map(|(wrapped, _)| wrapped.size(line_height).height)
        .sum();
    ((height / line_height).round() as usize).max(1)
}

/// Per-document, per-paint layout data that lets `measure_document`/
/// `paint_lines`/`offset_for_position`/`input_handler::bounds_for_range`
/// each touch only the lines they actually need, instead of every line in
/// the document — see this module's own notes on viewport-bounded
/// rendering. Built once per paint by `analyze_line_layout`, which first
/// makes sure the two caches below are up to date (cheaply, in the common
/// case — see [`RowHeightCache`]'s own doc comment) and then just wraps
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
    row_cache: Rc<RefCell<RowHeightCache>>,
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

    /// For the paint trace: the wrap width the cached heights were computed
    /// for (`None` while still unknown), the exact window, and how many
    /// corrections painting has had to make so far.
    pub(super) fn trace_state(&self) -> (Option<Pixels>, std::ops::Range<usize>, usize) {
        let rows = self.row_cache.borrow();
        let width = (rows.available_width < Pixels::MAX).then_some(rows.available_width);
        (width, rows.verified.clone(), rows.late_corrections)
    }

    /// Makes the row counts of `range` exact. A cold rebuild seeds ordinary
    /// lines with an over-estimate that painting only corrects line by line,
    /// *after* anything positioned from the estimates (a callout's
    /// background) has been drawn, so those backgrounds sat at the wrong
    /// height until the next repaint. Once the real width is known a one-row
    /// estimate is already exact (it never undercounts and a line has at
    /// least one row), so only multi-row lines are shaped, once per window.
    /// Before that (the first frame wraps at `Pixels::MAX`, so every line
    /// estimates as one row) every line in the window is shaped.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn verify_rows(
        &self,
        range: std::ops::Range<usize>,
        text: &str,
        cursor: usize,
        focused: bool,
        paint_width: Pixels,
        font: &Font,
        font_size: Pixels,
        text_color: Hsla,
        window: &mut Window,
    ) {
        let (verified, cached_width) = {
            let rows = self.row_cache.borrow();
            (rows.verified.clone(), rows.available_width)
        };
        if range.start >= verified.start && range.end <= verified.end {
            return;
        }
        let width_unknown = cached_width >= Pixels::MAX;
        let doc = self.doc_cache.borrow();
        let mut corrections = Vec::new();
        for i in range.clone().filter(|i| !verified.contains(i)) {
            if width_unknown || self.row_cache.borrow().row_count(i) > 1 {
                let rows = shaped_row_count_for_line(
                    text,
                    &doc,
                    i,
                    cursor,
                    focused,
                    self.line_height,
                    font,
                    font_size,
                    text_color,
                    paint_width,
                    window,
                );
                corrections.push((i, rows));
            }
        }
        let mut rows = self.row_cache.borrow_mut();
        for (i, count) in corrections {
            rows.correct(i, count);
        }
        let touching = range.start <= verified.end && verified.start <= range.end;
        rows.verified = if touching && !verified.is_empty() {
            range.start.min(verified.start)..range.end.max(verified.end)
        } else {
            range
        };
    }

    /// The width the cached row counts were built for.
    pub(super) fn rows_built_for(&self) -> Pixels {
        self.row_cache.borrow().available_width
    }

    /// Rebuilds every row count for `width`; see the call in `render_editor`.
    pub(super) fn rebuild_rows_for(
        &self,
        text: &str,
        (cursor, focused): (usize, bool),
        width: Pixels,
        char_width: Pixels,
        (font, font_size, text_color): (&Font, Pixels, Hsla),
        window: &mut Window,
    ) {
        let doc = self.doc_cache.borrow();
        self.row_cache.borrow_mut().rebuild(
            text, &doc, cursor, focused, width, char_width, font, font_size, text_color, window,
        );
    }

    /// The line containing buffer byte offset `cursor`.
    pub(super) fn line_at(&self, cursor: usize) -> usize {
        self.doc_cache.borrow().line_at(cursor)
    }

    /// Upgrades line `i`'s cached row count to `real_rows` — see
    /// `RowHeightCache::correct`'s own doc comment. `&self`, not `&mut
    /// self`: mutates through `row_cache`'s `RefCell`, the same interior-
    /// mutability pattern [`Self::cumulative_at`] already relies on.
    pub(super) fn correct_row_count(&self, i: usize, real_rows: usize) {
        let mut rows = self.row_cache.borrow_mut();
        if i < rows.line_count() && rows.row_count(i) != real_rows as i64 {
            rows.late_corrections += 1;
        }
        rows.correct(i, real_rows);
    }

    /// Binary search over [`Self::cumulative_at`], matching `[Pixels]::
    /// partition_point`'s semantics (the first index, in `0..=line_count()`,
    /// for which `pred` is false) exactly — reimplemented here rather than
    /// materializing the cumulative sequence into a slice (which would
    /// reintroduce the O(document) cost incremental caching exists to
    /// remove), since `RowHeightCache`'s own `Fenwick` tree only supports
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

/// The render parameters [`analyze_line_layout`] needs, computed the same
/// way both `render_editor` (every paint) and `Editor::record_edit`
/// (every edit, so [`RowHeightCache`] can be patched
/// synchronously right there rather than deferred to the next paint) need
/// them — pulled out into one function once a second call site needed the
/// exact same computation `render_editor` already did inline.
pub(super) struct RenderParams {
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) text_color: Hsla,
    pub(super) available_width: Pixels,
    pub(super) line_height: Pixels,
}

pub(super) fn render_params(
    editor: &Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> RenderParams {
    let text_style = window.text_style();
    // The note editor's own font, not the ambient `text_style`'s — only the
    // bundled mono family has real Bold/Italic/BoldItalic faces (the Sans UI
    // family only has Regular/Italic), which Live Preview's bold/italic runs
    // need.
    let mut font = text_style.font();
    font.family = gpui_kit::component::Theme::global(cx)
        .mono_font_family
        .clone();
    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let text_color = ThemeEngine::color(cx, Role::Text);
    // The editor's own on-screen width isn't known yet before the first
    // layout — `scroll_handle`'s tracked container bounds (the same "last
    // completed layout" value `autoscroll_to_cursor` already relies on)
    // stands in for it, falling back to "don't wrap" (`Pixels::MAX`) only
    // before that first layout, self-correcting the moment it completes.
    let available_width = {
        let width = editor.scroll_handle.bounds().size.width;
        if width > px(0.) { width } else { Pixels::MAX }
    };
    let line_height = window.line_height();
    RenderParams {
        font,
        font_size,
        text_color,
        available_width,
        line_height,
    }
}

/// Makes sure `row_cache` is up to date (a cheap active-line sync in the
/// common case, a full rebuild only on the first call or an actual
/// render-context change — see [`RowHeightCache`]'s own doc comment) and
/// returns a [`LineLayout`] wrapping it alongside `doc_cache`. Replaces what
/// used to be an unconditional O(document) pass here on every single paint.
#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_line_layout(
    text: &str,
    doc_cache: &Rc<RefCell<DocumentCache>>,
    row_cache: &Rc<RefCell<RowHeightCache>>,
    cursor: usize,
    focused: bool,
    available_width: Pixels,
    line_height: Pixels,
    font: &Font,
    font_size: Pixels,
    text_color: Hsla,
    window: &mut Window,
) -> LineLayout {
    let char_width = mono_char_width(font, font_size, text_color, window);
    {
        let doc = doc_cache.borrow();
        let mut rows = row_cache.borrow_mut();
        if rows.needs_full_rebuild(&doc, available_width, char_width) {
            rows.rebuild(
                text,
                &doc,
                cursor,
                focused,
                available_width,
                char_width,
                font,
                font_size,
                text_color,
                window,
            );
        } else {
            rows.sync_active_line(
                text, &doc, cursor, focused, char_width, font, font_size, text_color, window,
            );
        }
    }
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

/// Computes the whole document's total rendered height (estimated — see
/// [`analyze_line_layout`]) and the cursor's own line's accumulated top-Y
/// offset and height — needed by `render_editor` *before* it can construct
/// its element tree (the outer `div`'s own `.h(...)` sizing, and
/// `autoscroll_to_cursor`'s inputs), which is before the canvas paint
/// closure where `paint_lines`/`offset_for_position` do their own (now
/// viewport-bounded, real) shaping. No shaping happens in this function at
/// all — everything here is a handful of O(log n)/O(callouts.len()) lookups
/// into `layout`, which (via `analyze_line_layout`) is almost always already
/// up to date before this paint even starts.
fn measure_document(
    layout: &LineLayout,
    cursor: usize,
    line_height: Pixels,
) -> (Pixels, Pixels, Pixels) {
    let total = layout.cumulative_at(layout.line_count());

    let cursor_line = layout.line_at(cursor);
    let cursor_top = layout.cumulative_at(cursor_line);
    let cursor_height = layout.height(cursor_line).max(line_height);

    (total, cursor_top, cursor_height)
}

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
        | InlineStyle::ListMarker => {}
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
        | InlineStyle::Strikethrough => base,
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
pub(super) fn build_display_lines(
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
    let mut display_lines: Vec<DisplayLine> = match role {
        LineRole::FenceContent => vec![build_display_line(line, true)],
        LineRole::FenceDelimiter if !active => vec![hidden_line(line)],
        LineRole::FenceDelimiter => vec![build_display_line(line, active)],
        LineRole::Plain if table_active && !matches!(table_kind, TableLineKind::None) => {
            vec![build_display_line(line, true)]
        }
        LineRole::Plain if callout_is_header => vec![build_callout_header_line(line, active)],
        LineRole::Plain => match table_kind {
            TableLineKind::None if !active && is_thematic_break(line) => vec![hidden_line(line)],
            TableLineKind::None => vec![build_display_line_in_context(line, active, block_ctx)],
            TableLineKind::Delimiter(_) => vec![hidden_line(line)],
            TableLineKind::Row(table) => {
                let max_chars =
                    table_row_char_budget(available_width, font, font_size, text_color, window);
                build_table_row(line, &table.columns, max_chars)
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
pub(super) fn wrapped_row_count(
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
pub(super) fn shape_display_line(
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
                        color_for_style(seg.style, &colors, callout)
                    };
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
                .shape_text(
                    display_line.display_text.clone().into(),
                    font_size,
                    &runs,
                    wrap_width,
                    None,
                )
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
pub(super) fn mono_char_width(
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
fn fit_with_indent(available_width: Pixels, char_width: Pixels, level: u8) -> usize {
    if char_width <= px(0.) {
        return 0;
    }
    ((available_width - indent_px(level)).max(px(1.)) / char_width) as usize
}

/// How many monospace characters of `char_width` fit in `available_width`;
/// `None` if either is non-positive. Pure, so a loop over many lines can
/// measure `char_width` once ([`mono_char_width`] shapes text).
fn char_budget(available_width: Pixels, char_width: Pixels) -> Option<usize> {
    if available_width <= px(0.) || char_width <= px(0.) {
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
/// exact same builder methods already used throughout `app.rs`; `canvas`
/// only needs to own the actual text/cursor painting and (since it has
/// `bounds` in scope precisely when painting, unlike an event registered
/// outside of paint) the mouse hit-testing math.
pub(super) fn render_editor(
    editor: &Editor,
    editor_entity: Entity<Editor>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> impl IntoElement {
    let render_start = crate::debug_server::frames::now();
    // Read straight from `editor` (the `&mut self` `Editor::render` already
    // has), not `editor_entity.read(cx)` — this function runs *as* that
    // entity's own render, so reading through the entity handle here panics
    // ("cannot read while already being updated"). `editor_entity` is only
    // for the mouse closures below, which run later, once rendering has
    // finished and reading/updating the entity is safe again.
    let buffer = editor.buffer();
    let text = buffer.text();
    let cursor = buffer.cursor();
    let selection = buffer.selection();
    let focus_handle = editor.focus_handle.clone();
    let scroll_handle = editor.scroll_handle.clone();
    let last_autoscrolled_cursor = editor.last_autoscrolled_cursor.clone();
    // Reads the persisted `DocumentCache` (see that module's own doc
    // comment) instead of re-running all four classify_* passes per paint.
    // The paint and mouse closures below are `'static`, so they reach the
    // text and cache through `editor_entity` when they run rather than
    // holding per-frame copies (which cost ~25MB per frame on the stress
    // fixture, the same on every scroll as on every keystroke).
    let document_cache = editor.document_cache.clone();
    let row_height_cache = editor.row_height_cache.clone();

    let focused = focus_handle.is_focused(window);
    let params = render_params(editor, window, cx);
    let RenderParams {
        font,
        font_size,
        text_color,
        available_width,
        line_height,
    } = params;

    // Makes sure `row_height_cache` is up to date (almost always a cheap
    // active-line check — see `RowHeightCache`'s own doc comment, and
    // `Editor::record_edit` for the edit-triggered half of
    // keeping it that way) and wraps it alongside `document_cache` into a
    // `LineLayout`, feeding `measure_document`'s sizing and `paint_lines`/
    // `offset_for_position`/`register_mouse_handling`'s viewport-bounded
    // windowing below.
    let layout = analyze_line_layout(
        text,
        &document_cache,
        &row_height_cache,
        cursor,
        focused,
        available_width,
        line_height,
        &font,
        font_size,
        text_color,
        window,
    );
    let (total_height, cursor_top, cursor_height) = measure_document(&layout, cursor, line_height);
    let render_us = crate::debug_server::frames::micros_since(render_start);

    div()
        .id("editor")
        .test_support()
        .w_full()
        .h(total_height)
        // At least as tall as the scroll pane, so a click anywhere below the
        // text still lands in the editor (and puts the cursor at the end).
        .min_h_full()
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
                    let frame_clock = crate::debug_server::frames::start();
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
                    let viewport = scroll_handle.bounds();
                    // `render` sized the rows from the previous layout's width, so a
                    // sidebar toggle reaches it one frame late. Layout is final here:
                    // rebuild for the real width before anything is placed from the
                    // rows, and `notify` so the next render sizes the scroll extent.
                    let real_width = viewport.size.width;
                    if real_width > px(0.) && layout.rows_built_for() != real_width {
                        editor_entity.update(cx, |editor, _| {
                            let char_width = mono_char_width(&font, font_size, text_color, window);
                            layout.rebuild_rows_for(
                                editor.buffer().text(),
                                (cursor, focused),
                                real_width,
                                char_width,
                                (&font, font_size, text_color),
                                window,
                            );
                        });
                        // A `notify` made while painting is dropped; defer it.
                        let entity = editor_entity.clone();
                        window.defer(cx, move |_, cx| entity.update(cx, |_, cx| cx.notify()));
                    }
                    let view = Rc::new(FrameView {
                        layout: layout.clone(),
                        cursor,
                        focused,
                        bounds,
                        viewport,
                        line_height,
                        font: font.clone(),
                        font_size,
                        text_color,
                    });
                    editor_entity.update(cx, |editor, cx| {
                        if editor.hovering_link {
                            window.set_window_cursor_style(CursorStyle::PointingHand);
                        }
                        let doc = editor.document_cache.borrow();
                        paint_lines(
                            editor.buffer().text(),
                            &editor.link_table,
                            &view,
                            &doc,
                            selection.clone(),
                            window,
                            cx,
                        );
                    });

                    register_mouse_handling(
                        editor_entity.clone(),
                        view.clone(),
                        focus_handle.clone(),
                        window,
                    );

                    // Only registers while `focus_handle` is actually focused
                    // (checked internally by `handle_input`) — installs this
                    // editor as the platform's text input target so typing
                    // and IME composition reach `input_handler.rs` instead of
                    // (or before falling through to) `.on_key_down`.
                    window.handle_input(
                        &focus_handle,
                        ElementInputHandler::new(bounds, editor_entity.clone()),
                        cx,
                    );
                    crate::debug_server::frames::finish(frame_clock, render_us);
                },
            )
            .size_full(),
        )
        .debug_name("editor")
}

/// Registers this frame's mouse listeners: press to focus and place the
/// cursor (collapsing any selection), drag (move while the button that
/// pressed inside these bounds is still down) to extend it, release to stop
/// dragging. `window.on_mouse_event` listeners last one frame each — this
/// runs on every paint, same as the painting itself.
fn register_mouse_handling(
    editor_entity: Entity<Editor>,
    view: Rc<FrameView>,
    focus_handle: FocusHandle,
    window: &mut Window,
) {
    let bounds = view.bounds;
    window.on_mouse_event({
        let editor_entity = editor_entity.clone();
        let view = view.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !bounds.contains(&event.position)
                // A raw `window.on_mouse_event` listener isn't hitbox-aware
                // — it fires purely off `bounds.contains(...)`, regardless
                // of a `Dialog`'s own occluding backdrop sitting visually
                // on top (GPUI's `occlude()` only gates `Hitbox::is_hovered`-
                // based convenience wrappers like `.on_click`, never the raw
                // mouse-listener dispatch loop). Without this guard, a click
                // on a dialog that happens to be positioned over the editor
                // (e.g. Settings) also starts a drag-select here.
                || window.has_active_dialog(cx)
            {
                return;
            }
            let offset = offset_for_position(editor_entity.read(cx), &view, event.position, window);
            // On a rendered line a plain click follows a link; on the cursor's
            // own (raw) line only Ctrl/Cmd+click does, so the text stays editable.
            let follow = {
                let editor = editor_entity.read(cx);
                (event.modifiers.secondary() || !editor.on_cursor_line(offset))
                    .then(|| editor.link_event_at(offset))
                    .flatten()
            };
            if let Some(follow) = follow {
                editor_entity.update(cx, |_, cx| cx.emit(follow));
                return;
            }
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
            if phase != DispatchPhase::Bubble {
                return;
            }
            let dialog = window.has_active_dialog(cx);
            if !editor_entity.read(cx).is_dragging {
                // Not selecting: only the pointer cursor over a followable
                // link needs working out. Shaping lines to find the offset
                // is skipped for notes without links and outside the editor.
                let probe = !dialog
                    && bounds.contains(&event.position)
                    && editor_entity.read(cx).link_count() > 0;
                let hovering = probe && {
                    let editor = editor_entity.read(cx);
                    let offset = offset_for_position(editor, &view, event.position, window);
                    editor.follows_link_at(offset, event.modifiers.secondary())
                };
                if editor_entity.read(cx).hovering_link != hovering {
                    editor_entity.update(cx, |editor, cx| {
                        editor.hovering_link = hovering;
                        cx.notify();
                    });
                }
                return;
            }
            if dialog {
                return;
            }
            let offset = offset_for_position(editor_entity.read(cx), &view, event.position, window);
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

/// Everything a frame's painting and mouse hit-testing share, captured once
/// at paint time: the line layout and where the editor, its viewport and the
/// cursor are, with the font the lines are shaped in.
struct FrameView {
    layout: LineLayout,
    cursor: usize,
    focused: bool,
    bounds: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    line_height: Pixels,
    font: Font,
    font_size: Pixels,
    text_color: Hsla,
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
/// Only reshapes lines within `visible_line_range` (a click outside that —
/// shouldn't happen in practice, since clicks land inside the visible
/// viewport by construction — falls back to that range's own nearest
/// edge rather than scanning the rest of the document).
fn offset_for_position(
    editor: &Editor,
    view: &FrameView,
    position: Point<Pixels>,
    window: &mut Window,
) -> usize {
    let text = editor.buffer().text();
    let doc = editor.document_cache.borrow();
    let &FrameView {
        ref layout,
        cursor,
        focused,
        bounds,
        viewport,
        line_height,
        ref font,
        font_size,
        text_color,
    } = view;
    let relative_y = (position.y - bounds.origin.y).max(px(0.));
    let range = visible_line_range(layout, bounds, viewport);
    if range.is_empty() {
        return 0;
    }

    let mut y_offset = layout.cumulative_at(range.start);
    for i in range.clone() {
        let line_text = &text[layout.line_range(i)];
        let results = shape_display_line(
            line_text,
            &LineState::of(&doc, i, cursor, focused),
            font,
            font_size,
            text_color,
            bounds.size.width,
            window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap, each entry then its own (unwrapped, since
        // already sized to fit) sub-row of that row — same click-then-find
        // the right sub-row logic, just one level deeper.
        let multi_entry = results.len() > 1;
        for (wrapped, display_line) in &results {
            let height = wrapped.size(line_height).height;
            if relative_y < y_offset + height {
                let relative_x =
                    (position.x - bounds.origin.x - indent_px(display_line.indent.level()))
                        .max(px(0.));
                let local_y = (relative_y - y_offset).max(px(0.));
                let display_offset = match wrapped
                    .closest_index_for_position(point(relative_x, local_y), line_height)
                {
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
                return layout.line_range(i).start + raw_offset;
            }
            y_offset += height;
        }
    }
    // Past the visible window's last line (clicked below all rendered
    // text): land at the end of that window's last line, not the
    // document's — the window already covers the whole viewport plus
    // overscan, so this is only reachable by clicking well past the
    // rendered content.
    if range.end >= layout.line_count() {
        text.len()
    } else {
        layout.line_range(range.end - 1).end
    }
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
fn paint_table_grid_lines(paint: &TablePaint, bottom: Pixels, color: Hsla, window: &mut Window) {
    for &x in &paint.column_x {
        window.paint_quad(fill(
            Bounds {
                origin: point(x, paint.top),
                size: size(TABLE_LINE_WIDTH, bottom - paint.top),
            },
            color,
        ));
    }
    let left = paint.column_x.first().copied().unwrap_or(px(0.));
    let right = paint.column_x.last().copied().unwrap_or(px(0.));
    let width = right - left;
    window.paint_quad(fill(
        Bounds {
            origin: point(left, paint.top),
            size: size(width, TABLE_LINE_WIDTH),
        },
        color,
    ));
    window.paint_quad(fill(
        Bounds {
            origin: point(left, bottom - TABLE_LINE_WIDTH),
            size: size(width, TABLE_LINE_WIDTH),
        },
        color,
    ));
    if let Some(divider_y) = paint.divider_y {
        window.paint_quad(fill(
            Bounds {
                origin: point(left, divider_y),
                size: size(width, TABLE_LINE_WIDTH),
            },
            color,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_lines(
    text: &str,
    links: &LinkTable,
    view: &FrameView,
    doc: &DocumentCache,
    selection: Option<std::ops::Range<usize>>,
    window: &mut Window,
    cx: &mut App,
) {
    let (line_roles, tables, callouts) = (doc.line_roles(), doc.tables(), doc.callouts());
    let &FrameView {
        ref layout,
        cursor,
        focused: show_cursor,
        bounds,
        viewport,
        line_height,
        ref font,
        font_size,
        text_color,
    } = view;
    let palette = ThemeEngine::global(cx).palette().clone();
    let inline = InlineColors::from_palette(&palette);
    let rule = palette.color(Role::BorderStrong);
    let selection_fill = palette.with_alpha(Role::Selection, Alpha::Selection);
    let cursor_color = palette.color(Role::Cursor);
    let range = visible_line_range(layout, bounds, viewport);
    let mut trace = PaintTrace::start_if_enabled(|| {
        let (cached_width, verified, late_corrections_before) = layout.trace_state();
        let rect = |b: Bounds<Pixels>| {
            (
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height),
            )
        };
        FrameInfo {
            bounds: rect(bounds),
            viewport: rect(viewport),
            cursor,
            focused: show_cursor,
            range: range.clone(),
            total_height: f32::from(layout.cumulative_at(layout.line_count())),
            cached_width: cached_width.map(f32::from),
            verified,
            late_corrections_before,
        }
    });

    // Every callout's background tint + accent bar, painted *before* any
    // line's text — unlike `TablePaint`'s grid lines (thin enough that
    // painting them after a row's own text is fine), a full-width
    // background fill painted afterward would paint right over and hide
    // that text. `callout_ranges` (from `measure_document`'s own
    // `layout.cumulative_at` lookups) gives each callout's already-known
    // top/bottom extent without needing to have visited its lines at all, so
    // this runs as its own pass, skipping any callout whose range doesn't
    // intersect the viewport at all (cheap, and keeps this pass bounded
    // even for a document with a huge number of callouts).
    layout.verify_rows(
        range.clone(),
        text,
        cursor,
        show_cursor,
        bounds.size.width,
        font,
        font_size,
        text_color,
        window,
    );
    // Callouts never overlap and are ordered, so the visible ones are a
    // contiguous run.
    let first_visible = callouts.partition_point(|c| c.end_line <= range.start);
    for callout in callouts[first_visible..]
        .iter()
        .take_while(|c| c.start_line < range.end)
    {
        // Past the verified window the heights are estimates; the band is
        // off-screen there anyway, so stop where they begin.
        let (top, bottom) = (
            layout.cumulative_at(callout.start_line),
            layout.cumulative_at(callout.end_line.min(range.end)),
        );
        if let Some(trace) = trace.as_mut() {
            trace.band(
                callout.start_line..callout.end_line.min(range.end),
                f32::from(top),
                f32::from(bottom),
                callout.kind,
            );
        }
        let color = palette.color(callout_role(callout.kind));
        let background = palette.with_alpha(callout_role(callout.kind), Alpha::CalloutBackground);
        let top = bounds.origin.y + top;
        let bottom = bounds.origin.y + bottom;
        window.paint_quad(fill(
            Bounds {
                origin: point(bounds.origin.x, top),
                size: size(bounds.size.width, bottom - top),
            },
            background,
        ));
        window.paint_quad(fill(
            Bounds {
                origin: point(bounds.origin.x + QUOTE_BAR_INSET, top),
                size: size(CALLOUT_BAR_WIDTH, bottom - top),
            },
            color,
        ));
    }

    if range.is_empty() {
        return;
    }

    let mut y_offset = layout.cumulative_at(range.start);
    // Accumulated across a table's own lines, then painted and cleared
    // once its last line is reached — see `TablePaint`/`paint_table_grid_
    // lines`. Never started at all for a table that's rendering raw
    // (`table_active`), matching every other construct's own "border
    // disappears while active" behavior. Pre-seeded from the estimate
    // (`layout.cumulative_at`) if this window starts *inside* a table whose
    // header row is off-screen — the main loop below only ever discovers
    // a table's start by actually visiting its header row, which doesn't
    // happen if that row is outside `range`.
    let mut table_paint: Option<TablePaint> = None;
    if let Some(idx) = table_index_at(tables, range.start) {
        let table = &tables[idx];
        if table.start_line < range.start {
            let max_chars =
                table_row_char_budget(bounds.size.width, font, font_size, text_color, window);
            let widths = table_preview::resolve_widths(&table.columns, max_chars);
            let char_width = mono_char_width(font, font_size, text_color, window);
            // A table's own indent is always `None`, so its header row's
            // `line_origin.x` (what the real `is_first_row` branch below
            // would use) is always just `bounds.origin.x` — safe to use
            // directly even though that row itself is off-screen.
            let column_x: Vec<Pixels> = table_preview::column_divider_char_offsets(&widths)
                .into_iter()
                .map(|offset| bounds.origin.x + char_width * offset as f32)
                .collect();
            let delimiter_line = table.start_line + 1;
            let divider_y = (delimiter_line < range.start).then(|| {
                bounds.origin.y
                    + layout.cumulative_at(delimiter_line)
                    + layout.height(delimiter_line) / 2.
            });
            table_paint = Some(TablePaint {
                top: bounds.origin.y + layout.cumulative_at(table.start_line),
                divider_y,
                column_x,
            });
        }
    }

    for i in range.clone() {
        let line_range = layout.line_range(i);
        let line_text = &text[line_range.clone()];
        let broken: Vec<Range<usize>> = links.broken_on_line(i).collect();
        let state = LineState {
            broken: &broken,
            ..LineState::of(doc, i, cursor, show_cursor)
        };
        let callout = state.callout;
        let results = shape_display_line(
            line_text,
            &state,
            font,
            font_size,
            inline,
            bounds.size.width,
            window,
        );
        // Usually one entry; more than one only for a table row whose
        // cells had to wrap (see `shape_display_line`'s own doc comment).
        let total_height: Pixels = results
            .iter()
            .map(|(wrapped, _)| wrapped.size(line_height).height)
            .sum();
        // Upgrades a cold-rebuild estimate (see `RowHeightCache::rebuild`)
        // to the truth using the real shaping this line's painting already
        // needed — zero extra cost, and idempotent (a no-op once this line
        // has already been corrected, e.g. by an earlier viewing or an
        // edit). `.round()` because `total_height` is always an exact
        // multiple of `line_height` in principle, but float division can
        // land a hair off either side of the actual integer.
        layout.correct_row_count(i, (total_height / line_height).round() as usize);
        if let Some(trace) = trace.as_mut() {
            let tag = match (callout, table_line_kind(tables, i), line_roles[i]) {
                (Some(c), ..) if c.is_header(i) => "callout-header",
                (Some(_), ..) => "callout",
                (None, TableLineKind::Row(_) | TableLineKind::Delimiter(_), _) => "table",
                (None, _, LineRole::FenceContent) => "code",
                _ => "plain",
            };
            trace.line(
                i,
                f32::from(y_offset),
                (total_height / line_height).round() as usize,
                f32::from(total_height),
                tag,
                line_text,
            );
            trace.set_late_corrections(layout.trace_state().2);
        }
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
        if !state.active
            && matches!(line_roles[i], LineRole::Plain)
            && matches!(table_line_kind(tables, i), TableLineKind::None)
            && is_thematic_break(line_text)
        {
            let mid_y = bounds.origin.y + y_offset + total_height / 2.;
            window.paint_quad(fill(
                Bounds {
                    origin: point(bounds.origin.x, mid_y),
                    size: size(bounds.size.width, TABLE_LINE_WIDTH),
                },
                rule,
            ));
        }

        if !state.table_active {
            // `table_kind` was already moved into `shape_display_line`
            // above — cheap to resolve again (a plain array lookup) rather
            // than restructuring this loop just to keep the first value
            // alive.
            let table_kind_for_geometry = table_line_kind(tables, i);
            if let TableLineKind::Row(table) | TableLineKind::Delimiter(table) =
                &table_kind_for_geometry
            {
                if table.is_first_row(i) {
                    let max_chars = table_row_char_budget(
                        bounds.size.width,
                        font,
                        font_size,
                        text_color,
                        window,
                    );
                    let widths = table_preview::resolve_widths(&table.columns, max_chars);
                    let char_width = mono_char_width(font, font_size, text_color, window);
                    let column_x = table_preview::column_divider_char_offsets(&widths)
                        .into_iter()
                        .map(|offset| line_origin.x + char_width * offset as f32)
                        .collect();
                    table_paint = Some(TablePaint {
                        top: bounds.origin.y + y_offset,
                        divider_y: None,
                        column_x,
                    });
                }
                if table.is_delimiter(i)
                    && let Some(paint) = table_paint.as_mut()
                {
                    paint.divider_y = Some(bounds.origin.y + y_offset + total_height / 2.);
                }
                if table.is_last_line(i)
                    && let Some(paint) = table_paint.take()
                {
                    paint_table_grid_lines(
                        &paint,
                        bounds.origin.y + y_offset + total_height,
                        rule,
                        window,
                    );
                }
            }
        }

        // A callout's own accent bar (painted in the pre-pass above, at
        // this same gutter position since a callout is always depth-1)
        // already covers this line — painting the plain muted bar too
        // would paint right over it, hiding the color.
        if callout.is_none()
            && let LineIndent::Blockquote(depth) = results[0].1.indent
        {
            for level in 0..depth {
                let bar_x = bounds.origin.x + indent_px(level) + QUOTE_BAR_INSET;
                window.paint_quad(fill(
                    Bounds {
                        origin: point(bar_x, bounds.origin.y + y_offset),
                        size: size(QUOTE_BAR_WIDTH, total_height),
                    },
                    rule,
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
                    Bounds {
                        origin: line_origin,
                        size: size(full_row_width, total_height),
                    },
                    selection_fill,
                ));
            }
            let mut row_y = y_offset;
            for (wrapped, _) in &results {
                let _ = wrapped.paint(
                    bounds.origin + point(indent_px(indent_level), row_y),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                row_y += wrapped.size(line_height).height;
            }
            if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
                window.paint_quad(fill(
                    Bounds {
                        origin: line_origin,
                        size: size(px(1.), line_height),
                    },
                    cursor_color,
                ));
            }
        } else {
            let (wrapped, display_line) = &results[0];

            if let Some(selection) = &selection {
                let start = selection.start.max(line_range.start);
                let end = selection.end.min(line_range.end);
                if start < end {
                    let start_pos = wrapped
                        .position_for_index(
                            display_line.display_offset_for_raw(start - line_range.start),
                            line_height,
                        )
                        .unwrap_or(point(px(0.), px(0.)));
                    let end_pos = wrapped
                        .position_for_index(
                            display_line.display_offset_for_raw(end - line_range.start),
                            line_height,
                        )
                        .unwrap_or(point(full_row_width, total_height - line_height));
                    let start_row = (start_pos.y / line_height) as i32;
                    let end_row = (end_pos.y / line_height) as i32;
                    if start_row == end_row {
                        window.paint_quad(fill(
                            Bounds {
                                origin: line_origin + start_pos,
                                size: size(end_pos.x - start_pos.x, line_height),
                            },
                            selection_fill,
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
                                    selection_fill,
                                ));
                            }
                        }
                    }
                }
            }

            let _ = wrapped.paint(line_origin, line_height, TextAlign::Left, None, window, cx);

            if show_cursor && cursor >= line_range.start && cursor <= line_range.end {
                let pos = wrapped
                    .position_for_index(
                        display_line.display_offset_for_raw(cursor - line_range.start),
                        line_height,
                    )
                    .unwrap_or(point(px(0.), px(0.)));
                window.paint_quad(fill(
                    Bounds {
                        origin: line_origin + pos,
                        size: size(px(1.), line_height),
                    },
                    cursor_color,
                ));
            }
        }

        y_offset += total_height;
    }
}

// Deliberately not `use super::*`/`use gpui_kit::*` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows plain `#[test]` and makes `#[gpui_kit::test]`'s own expansion
// recurse infinitely (see the project-wide note on this in `app.rs`).
#[cfg(test)]
mod tests {
    use super::super::block_preview::LineRole;
    use super::super::live_preview::LineIndent;
    use super::super::table_preview::classify_tables;
    use super::super::test_support::apply_edit;
    use super::super::{Editor, InsertTab};
    use super::LineState;
    use super::{TableLineKind, is_active_line, row_count_for_line, shape_display_line};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, EntityInputHandler, KeyBinding, Pixels, Point,
        TestAppContext, WindowOptions, point, px,
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
    fn a_table_renders_as_a_grid_until_the_cursor_enters_it_then_renders_raw(
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
            // Columns resolve to width 5 ("Alice" is the widest cell in
            // column 0) and width 3 ("Age"/"Age" is widest in column 1).
            // The blank gaps at the border positions aren't drawn as text —
            // `element.rs`'s `paint_table_grid_lines` paints real vector
            // lines over them instead, see that function's own doc comment.
            assert_eq!(grid.display_text, "  Name    Age  ");

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
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
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
            // line verbatim anyway (see `live_preview`'s own tests) — the
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
    fn row_counts(rows: &super::RowHeightCache) -> Vec<i64> {
        (0..rows.line_count())
            .map(|i| rows.prefix_sum(i + 1) - rows.prefix_sum(i))
            .collect()
    }

    /// The true, never-estimated row count for every line, computed
    /// directly via `row_count_for_line` (the same function `RowHeightCache
    /// ::patch_lines`/`sync_active_line` already always use) — the oracle
    /// every property test below compares an incrementally-maintained
    /// cache against. Deliberately *not* `RowHeightCache::rebuild`'s own
    /// output: `rebuild` itself now estimates for non-table lines (see its
    /// own doc comment) and is only guaranteed exact for whichever single
    /// line is currently active, so it stopped being a valid full-document
    /// "ground truth" the moment that landed — this helper is what still
    /// is one.
    #[allow(clippy::too_many_arguments)]
    fn exact_row_counts(
        text: &str,
        doc: &super::super::document_cache::DocumentCache,
        cursor: usize,
        focused: bool,
        char_width: Pixels,
        font: &gpui_kit::Font,
        font_size: Pixels,
        text_color: gpui_kit::Hsla,
        available_width: Pixels,
        window: &mut gpui_kit::Window,
    ) -> Vec<i64> {
        (0..doc.line_count())
            .map(|i| {
                super::row_count_for_line(
                    text,
                    doc,
                    i,
                    cursor,
                    focused,
                    char_width,
                    font,
                    font_size,
                    text_color,
                    available_width,
                    window,
                ) as i64
            })
            .collect()
    }

    /// The property [`super::RowHeightCache::patch_lines`]/[`super::
    /// RowHeightCache::rebuild`] actually rely on: after any sequence of
    /// edits (including line-count-changing ones, which force a fallback
    /// full rebuild — see `needs_full_rebuild`), a cache kept incrementally
    /// in sync edit-by-edit must have exactly the same per-line row counts
    /// as [`exact_row_counts`] computes directly from scratch on the final
    /// text — the same "fuzz against an oracle" discipline `document_
    /// cache.rs`'s own property tests already use, which is what actually
    /// caught every real bug in *that* module; this is the matching test
    /// for the row-height cache this module adds on top of it. Compared
    /// against `exact_row_counts` specifically, not a second `RowHeightCache
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
        use super::super::document_cache::DocumentCache;
        use super::{RowHeightCache, mono_char_width};

        let mut text = String::from(
            "intro\n```\ncode line one\ncode line two\n```\nplain\n> quote\n- item\n  - nested\nmore plain text here\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |\nafter table\n# Heading\nFinal line",
        );
        let (window, _editor) = open_editor(cx, "");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let text_style = window.text_style();
            let font = text_style.font();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let text_color = text_style.color;
            let available_width = px(400.);
            let char_width = mono_char_width(&font, font_size, text_color, window);

            let mut doc_cache = DocumentCache::new(&text);
            let mut row_cache = RowHeightCache::empty();
            row_cache.rebuild(
                &text,
                &doc_cache,
                0,
                false,
                available_width,
                char_width,
                &font,
                font_size,
                text_color,
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
                let patch = doc_cache.record_edit(&new_text, edit);
                if let Some(patch) = &patch {
                    row_cache.resize_for(patch, &doc_cache);
                }
                match patch.map(|p| p.touched) {
                    Some(range)
                        if !row_cache.needs_full_rebuild(
                            &doc_cache,
                            available_width,
                            char_width,
                        ) =>
                    {
                        row_cache.patch_lines(
                            &new_text, &doc_cache, range, 0, false, char_width, &font, font_size,
                            text_color, window,
                        );
                    }
                    _ => {
                        row_cache.rebuild(
                            &new_text,
                            &doc_cache,
                            0,
                            false,
                            available_width,
                            char_width,
                            &font,
                            font_size,
                            text_color,
                            window,
                        );
                    }
                }

                let oracle_doc = DocumentCache::new(&new_text);
                let expected = exact_row_counts(
                    &new_text,
                    &oracle_doc,
                    0,
                    false,
                    char_width,
                    &font,
                    font_size,
                    text_color,
                    available_width,
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

    /// The other half of [`super::RowHeightCache`]'s invalidation story:
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
        use super::super::document_cache::DocumentCache;
        use super::{RowHeightCache, mono_char_width};

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
            let char_width = mono_char_width(&font, font_size, text_color, window);

            let doc_cache = DocumentCache::new(&text);
            let mut row_cache = RowHeightCache::empty();
            row_cache.rebuild(
                &text,
                &doc_cache,
                0,
                true,
                available_width,
                char_width,
                &font,
                font_size,
                text_color,
                window,
            );

            // Move the cursor onto the bold line (line 1), then off it again
            // onto line 2 — each a pure cursor move, no edit at all.
            for cursor in [
                text.find(&long_bold).unwrap(),
                text.rfind("another").unwrap(),
                0,
            ] {
                row_cache.sync_active_line(
                    &text, &doc_cache, cursor, true, char_width, &font, font_size, text_color,
                    window,
                );

                let expected = exact_row_counts(
                    &text,
                    &doc_cache,
                    cursor,
                    true,
                    char_width,
                    &font,
                    font_size,
                    text_color,
                    available_width,
                    window,
                );

                assert_eq!(row_counts(&row_cache), expected, "cursor at {cursor}");
            }
        })
        .unwrap();
    }

    /// Proves the estimate-then-correct scheme (`RowHeightCache::rebuild`'s
    /// cheap seed, upgraded to the truth line-by-line as `paint_lines`
    /// actually corrects each one — see `RowHeightCache::correct`'s own
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
        use super::super::document_cache::DocumentCache;
        use super::{RowHeightCache, mono_char_width};

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
            let char_width = mono_char_width(&font, font_size, text_color, window);

            let doc_cache = DocumentCache::new(&text);
            let mut row_cache = RowHeightCache::empty();
            row_cache.rebuild(
                &text,
                &doc_cache,
                0,
                false,
                available_width,
                char_width,
                &font,
                font_size,
                text_color,
                window,
            );

            for i in 0..doc_cache.line_count() {
                let real = row_count_for_line(
                    &text,
                    &doc_cache,
                    i,
                    0,
                    false,
                    char_width,
                    &font,
                    font_size,
                    text_color,
                    available_width,
                    window,
                );
                row_cache.correct(i, real);
            }

            let expected = exact_row_counts(
                &text,
                &doc_cache,
                0,
                false,
                char_width,
                &font,
                font_size,
                text_color,
                available_width,
                window,
            );
            assert_eq!(row_counts(&row_cache), expected);
        })
        .unwrap();
    }
}

//! A persistent, edit-patched cache of the four whole-document Live Preview
//! classification passes (`preview::fence::classify_lines`,
//! `preview::classify_block_context`, `table::classify_tables`,
//! `preview::callout::classify_callouts`) — see ARCHITECTURE.md's own bullet on
//! incremental classification for why this exists: re-running all four from
//! scratch on every paint cost ~420ms combined on the 320K-line stress
//! fixture, re-paid on every keystroke, nowhere near the 16.66ms/60fps bar.
//!
//! **Real incremental paths landed for all four classifiers** —
//! `patch_block_preview`/`patch_callouts`/`patch_block_context` resume a
//! per-line state machine from the edited line; `patch_tables` (the one
//! classifier that isn't a per-line state machine at all — a table's
//! column widths are an aggregate over its whole row range) instead
//! rescans just the bounded window a table could possibly have been
//! affected in, not the whole document. See each function's own doc
//! comment.
//!
//! `line_index` (a [`Fenwick`] tree over per-line byte strides) is the
//! shared piece every incremental classifier needs and none of the four
//! `classify_*` passes themselves care about: mapping an edit's byte offset
//! to a line index in O(log n), without counting newlines from the start of
//! the document — see `lines/fenwick.rs`'s own doc comment for why that matters.
//! Every patch function below also walks forward from the edited line
//! using a running byte cursor and `text[cursor..].find('\n')` rather than
//! ever materializing the whole document's lines into a `Vec<&str>` first —
//! that materialization alone would be an O(document) pass, re-introducing
//! on every keystroke exactly the cost this cache exists to remove, even
//! though every patch function is only *looking* at the handful of lines it
//! actually walks.
//!
//! [`DocumentCache::record_edit`] also reports the union of every
//! classifier's own touched-line range back to its caller (`editor.rs`) —
//! not consumed by anything in this module, but the one piece of
//! information `element.rs`'s own `RowHeightCache` needs to patch just the
//! lines whose rendered row count could have changed, instead of
//! re-deriving the whole document's layout on every edit (see that
//! struct's own doc comment for the next classification-adjacent cost this
//! unblocks).

use super::table;
use std::ops::Range;
use std::sync::Arc;

use super::math::Macros;

mod edit;
mod macros;
mod math;
mod patch;
mod view;

use edit::EditSpan;
pub(in crate::text_editor) use edit::{EditPatch, TextEdit};
pub(in crate::text_editor) use math::MathLines;
use patch::*;
pub(in crate::text_editor) use view::ViewState;

use super::columns::{self, Columns, Placed};
#[cfg(test)]
use super::lines::count_lines;
use super::lines::{
    Fenwick, LINE_GROWTH_SLACK, line_strides, lines_from, lines_of, resize_vec, shift_line,
};
use super::preview::callout::{Callout, CalloutState};
use super::preview::fence::LineRole;
use super::preview::{self, LineContext, OpenBlock};
use super::table::{Table, is_pipe_row};

/// The persisted result of the four classify_* passes over the buffer's
/// current text — see this module's own doc comment.
pub(super) struct DocumentCache {
    line_roles: Vec<LineRole>,
    /// `fence_states[i]` is the fence state *entering* line `i` (so
    /// `fence_states[0]` is always `None`) — the small per-line value
    /// `patch_block_preview` resumes from and compares against to know
    /// when it can stop walking forward. Parallel to `line_roles`.
    fence_states: Vec<u64>,
    /// Per-line byte strides (`line_text.len() + 1` for every line but the
    /// last, which has no trailing `\n`) — lets every patch function below
    /// map an edit's byte offset to a line index in O(log n) instead of
    /// counting newlines from the start of the document. See `lines/fenwick.rs`.
    /// Shared across classifiers — there's only one true line layout.
    line_index: Fenwick,
    block_contexts: Vec<LineContext>,
    /// `open_block_states[i]` is the `OpenBlock` *entering* line `i`.
    /// Parallel to `block_contexts`, same role `fence_states` plays for
    /// `line_roles`.
    open_block_states: Vec<OpenBlock>,
    tables: Vec<Table>,
    /// The `$$` blocks, in line order.
    math_blocks: Vec<MathLines>,
    /// Lines holding a macro definition (`\newcommand`…), in order.
    macro_lines: Vec<usize>,
    /// The macros those lines define.
    macros: Arc<Macros>,
    callouts: Vec<Callout>,
    /// Reused by `patch_callouts` to hold the callouts the walk re-derives
    /// before they are spliced into `callouts`.
    callout_scratch: Vec<Callout>,
    /// `callout_states[i]` is the packed `CalloutState` *entering* line `i`.
    /// Parallel to `callout_for_line`.
    callout_states: Vec<u8>,
    /// Every `::: columns` marker line that is isolated and outside code, in
    /// line order: all the cross-line state the regions below need, so an edit
    /// rescans a few lines and re-pairs the markers (few) instead of the text.
    /// What the editor sets about how the text is shown; not derived from it.
    pub(in crate::text_editor) view: ViewState,
    column_markers: Vec<Placed>,
    /// The regions `column_markers` form.
    columns: Vec<Columns>,
}

impl DocumentCache {
    /// Builds a fully-populated cache for `text` — used both for a freshly
    /// opened note (eager, one-time cost, same decision Part 1's cold-open
    /// question already settled) and by every test that needs a cache
    /// without going through a full `Editor`.
    pub(super) fn new(text: &str) -> Self {
        let mut cache = Self {
            line_roles: Vec::new(),
            fence_states: Vec::new(),
            line_index: Fenwick::new(&[]),
            block_contexts: Vec::new(),
            open_block_states: Vec::new(),
            tables: Vec::new(),
            math_blocks: Vec::new(),
            macro_lines: Vec::new(),
            macros: Arc::new(Macros::default()),
            callouts: Vec::new(),
            callout_states: Vec::new(),
            callout_scratch: Vec::new(),
            view: ViewState::default(),
            column_markers: Vec::new(),
            columns: Vec::new(),
        };
        cache.recompute_all(text);
        cache
    }

    pub(super) fn line_roles(&self) -> &[LineRole] {
        &self.line_roles
    }

    pub(super) fn block_contexts(&self) -> &[LineContext] {
        &self.block_contexts
    }

    pub(super) fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub(super) fn callouts(&self) -> &[Callout] {
        &self.callouts
    }

    pub(super) fn columns(&self) -> &[Columns] {
        &self.columns
    }

    /// The index of the region `line` is in.
    pub(super) fn region_index_at(&self, line: usize) -> Option<usize> {
        let i = self.columns.partition_point(|r| r.end_line <= line);
        self.columns.get(i).filter(|r| r.start_line <= line)?;
        Some(i)
    }

    /// The region `line` is in and its split, when the region is laid out side
    /// by side (a width is set, every column fits and it is not shown as
    /// source). `None` otherwise.
    pub(super) fn side_by_side(&self, line: usize) -> Option<(Columns, columns::Split)> {
        let width = self.view.column_width()?;
        let index = self
            .region_index_at(line)
            .filter(|&i| Some(i) != self.view.raw_region())?;
        let mut region = self.columns[index];
        if let Some(shares) = self.view.preview_for(index, region.count) {
            region = region.with_shares(shares);
        }
        let split = match region.kind {
            columns::Kind::Aside => columns::aside_split(width, region.floats())?,
            columns::Kind::Columns => columns::split(width, &region.percent[..region.count])?,
        };
        Some((region, split))
    }

    /// Whether `line` is the open or close marker of a region shown as
    /// columns or stacked; none of a region shown as source.
    pub(super) fn is_column_frame(&self, line: usize) -> bool {
        let raw = self.view.raw_region().and_then(|i| self.columns.get(i));
        columns::is_frame_line(&self.columns, line)
            && !raw.is_some_and(|r| (r.start_line..r.end_line).contains(&line))
    }

    /// Folds or unfolds callout `index` for this session.
    pub(super) fn set_folded(&mut self, index: usize, folded: bool) {
        if let Some(c) = self.callouts.get_mut(index).filter(|c| c.foldable) {
            c.folded = folded;
        }
    }

    /// The document's current line count — the row-height cache's own
    /// line-count fingerprint (see `element.rs`'s `RowHeightCache`) compares
    /// against this to know when a line-count-changing edit (which this
    /// cache's own `line_roles.len()` already reflects, but which that
    /// *other* cache can't detect on its own) forces a full rebuild there
    /// too.
    pub(super) fn line_count(&self) -> usize {
        self.line_roles.len()
    }

    /// The byte offset line `line` begins at — the complement of
    /// [`Self::line_end_byte`].
    pub(super) fn line_start_byte(&self, line: usize) -> usize {
        self.line_index.prefix_sum(line) as usize
    }

    /// Full whole-document recompute of every classifier — the cold-open
    /// path, and the fallback whenever a patch function can't cheaply
    /// resolve a particular edit.
    pub(super) fn recompute_all(&mut self, text: &str) {
        // One scan for newlines; every pass reads the lines back out of the
        // strides, which then become the byte index.
        let strides = line_strides(text);
        self.recompute_block_preview(text, &strides);
        self.recompute_callouts(text, &strides);
        self.recompute_block_context(text, &strides);
        self.recompute_tables(text, &strides);
        self.recompute_columns(text, &strides);
        self.recompute_math(text);
        self.line_index = Fenwick::from_vec(strides);
        // Reads lines through the index, so only now.
        self.recompute_macros(text);
        self.cap_tails(text);
        // Spare room for Enter to grow into without reallocating.
        reserve_line_slack(self);
    }

    /// Patches the cache for a precisely-ranged edit: the real incremental
    /// path for every classifier. Returns the union of every classifier's
    /// own touched-line range (always including at least the lines whose
    /// raw *text* the edit itself changed, regardless of whether any
    /// classifier's output over them actually differs) — `element.rs`'s
    /// `RowHeightCache` consumes this to patch exactly those lines' row
    /// counts instead of re-deriving the whole document's layout. `None`
    /// means a line-count-changing edit forced a full recompute instead
    /// (see [`Self::edit_span`]), so every line is potentially dirty.
    pub(super) fn record_edit(&mut self, text: &str, edit: TextEdit) -> Option<EditPatch> {
        let Some(span) = self.edit_span(text, edit) else {
            self.recompute_all(text);
            return None;
        };

        let table_pre = self.table_pre(&span);
        let math_pre = self.math_pre(&span);
        self.resize_lines(&span);

        // Every line the edit's new text actually spans has different raw
        // text now, regardless of what any classifier below reports — this
        // is the floor of what's touched.
        let minimal = span.start_line..(span.last_new_line() + 1);

        let r1 = self.patch_block_preview(text, &span);
        let r2 = self.patch_callouts(text, &span);
        // `patch_tables` relies on `line_index` already reflecting the new
        // text's layout, which `patch_block_preview` above has already
        // guaranteed.
        let r3 = self.patch_block_context(text, &span);
        let r4 = self.patch_tables(text, &span, &table_pre);
        // Needs `r1`: a code fence opened or closed changes what is code below.
        let r5 = self.patch_columns(text, &span, &r1);
        let r6 = self.patch_math(text, &span, &math_pre, &r1);

        let mut touched = [minimal, r1, r2, r3, r4, r5, r6]
            .into_iter()
            .reduce(union_ranges)
            .unwrap();
        // A changed macro changes any formula of the note.
        if self.patch_macros(text, &span, &touched) {
            touched = 0..self.line_roles.len();
        }
        Some(EditPatch {
            touched: touched.start..touched.end.min(self.line_roles.len()),
            splice_at: span.splice_at(),
            line_delta: span.line_delta(),
        })
    }

    /// Resizes every per-line array (and the byte index) so entry `i` of
    /// each describes new line `i` for every line the edit left alone.
    /// Entries for the lines the edit replaced hold stale values the patch
    /// walks overwrite; tables and callouts after the edit also get their
    /// stored line numbers shifted.
    fn resize_lines(&mut self, span: &EditSpan) {
        let delta = span.line_delta();
        if delta == 0 {
            return;
        }
        let at = span.splice_at();

        // Tables the edit touched are dropped here; `patch_tables`
        // re-derives them from the extents `table_pre` saved. Tables after
        // the edit just move.
        let (first, end) = self.tables_touching(span);
        self.tables.drain(first..end);
        for t in &mut self.tables[first..] {
            t.start_line = shift_line(t.start_line, delta);
            t.end_line = shift_line(t.end_line, delta);
        }

        resize_vec(&mut self.line_roles, at, delta);
        resize_vec(&mut self.fence_states, at, delta);
        resize_vec(&mut self.block_contexts, at, delta);
        resize_vec(&mut self.open_block_states, at, delta);
        resize_vec(&mut self.callout_states, at, delta);
        self.line_index.resize(at, delta);
    }

    // The three `recompute_*` below stream each line's step straight into
    // its arrays: collecting a per-line result `Vec` first cost up to 15MB
    // of transient garbage on the stress fixture, which the allocator then
    // kept as retained-but-free memory.

    pub(super) fn recompute_block_preview(&mut self, text: &str, strides: &[i64]) {
        let n = strides.len();
        let mut roles = Vec::with_capacity(n + LINE_GROWTH_SLACK);
        let mut states = Vec::with_capacity(n + LINE_GROWTH_SLACK);
        let mut state = None;
        for line in lines_of(text, strides) {
            states.push(preview::fence::pack_fence(state));
            let (role, next) = preview::fence::step(state, line);
            roles.push(role);
            state = next;
        }
        self.line_roles = roles;
        self.fence_states = states;
    }

    pub(super) fn recompute_callouts(&mut self, text: &str, strides: &[i64]) {
        let n = strides.len();
        let mut callouts = Vec::new();
        let mut states = Vec::with_capacity(n + LINE_GROWTH_SLACK);
        let mut state = CalloutState::default();
        for (i, line) in lines_of(text, strides).enumerate() {
            states.push(state.pack());
            state = preview::callout::step(state, line, i, &mut callouts).1;
        }
        self.callout_states = states;
        self.callouts = callouts;
    }

    pub(super) fn recompute_block_context(&mut self, text: &str, strides: &[i64]) {
        let n = strides.len();
        let mut contexts = Vec::with_capacity(n + LINE_GROWTH_SLACK);
        let mut states = Vec::with_capacity(n + LINE_GROWTH_SLACK);
        let mut open = OpenBlock::None;
        for line in lines_of(text, strides) {
            states.push(open.clone());
            let (ctx, next) = preview::block_context_step(open, line);
            contexts.push(ctx);
            open = next;
        }
        self.block_contexts = contexts;
        self.open_block_states = states;
    }

    /// Needs the line roles (code fences), so runs after `recompute_block_preview`.
    pub(super) fn recompute_columns(&mut self, text: &str, strides: &[i64]) {
        let lines = lines_of(text, strides)
            .zip(&self.line_roles)
            .map(|(line, role)| (line, !matches!(role, LineRole::Plain)));
        self.column_markers = columns::place_markers(0, lines, true, true);
        self.columns = columns::derive(&self.column_markers, self.line_roles.len());
    }

    /// Sets where each float's text ends (see `Columns::cap_tail`): at the first
    /// blank line outside code past its budget, so no block is cut. Depends on
    /// the lines' content, not just the markers, so it runs after every edit.
    fn cap_tails(&mut self, text: &str) {
        let mut columns = std::mem::take(&mut self.columns);
        for region in columns.iter_mut().filter(|r| r.tail) {
            region.cap_tail(|line| {
                matches!(self.line_roles[line], LineRole::Plain)
                    && columns_blank(self.line_text_at(text, line))
            });
        }
        self.columns = columns;
    }

    pub(super) fn recompute_tables(&mut self, text: &str, strides: &[i64]) {
        self.tables = table::classify_table_list_from(lines_of(text, strides));
    }

    /// Every line's start byte, plus the document's length as a final entry
    /// (`line_count() + 1` values), in one O(n) sweep: line `i` is
    /// `starts[i]..starts[i + 1] - 1` (without its `\n`), except the last
    /// line, which runs to `starts[line_count()]`. Far cheaper than scanning
    /// the text for newlines or calling [`Self::line_start_byte`] per line.
    pub(super) fn line_index(&self) -> &Fenwick {
        &self.line_index
    }

    /// The byte offset right after line `line`'s own content, excluding
    /// its trailing `\n` (or the document's total length, for the actual
    /// last line, which has none) — the complement of [`Self::line_start_
    /// byte`].
    pub(super) fn line_end_byte(&self, line: usize) -> usize {
        let next_start = self.line_index.prefix_sum(line + 1) as usize;
        if line + 1 < self.line_roles.len() {
            next_start - 1
        } else {
            next_start
        }
    }

    /// `line`'s own text, sliced directly out of `text` via `line_index` —
    /// an O(log n) + O(line length) lookup, not a scan from the start of
    /// the document, usable for any line regardless of whether it's near
    /// the edit or not (unlike `lines_from`, which only walks forward from
    /// a known starting byte).
    fn line_text_at<'t>(&self, text: &'t str, line: usize) -> &'t str {
        let start = self.line_index.prefix_sum(line) as usize;
        &text[start..self.line_end_byte(line)]
    }

    /// The bytes `table` spans, from its first line's start to its last
    /// line's end. Derived from `line_index`, so it can't go stale.
    pub(super) fn table_byte_range(&self, table: &Table) -> Range<usize> {
        self.line_start_byte(table.start_line)..self.line_end_byte(table.end_line - 1)
    }

    /// The line index whose byte-stride range contains `byte_offset`,
    /// clamped to the last line if `byte_offset` is at or past the
    /// document's current total byte length (e.g. appending at the very
    /// end) — `line_index.find` itself only answers for offsets strictly
    /// inside the tree's total.
    pub(super) fn line_at(&self, byte_offset: usize) -> usize {
        if byte_offset as i64 >= self.line_index.total() {
            return self.line_roles.len().saturating_sub(1);
        }
        self.line_index.find(byte_offset as i64).unwrap_or(0)
    }
}

/// The smallest range containing both `a` and `b` — used by
/// [`DocumentCache::record_edit`] to union each classifier's own touched
/// range into one overall "these lines might need their row heights
/// recomputed" range.
fn union_ranges(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
}

/// Keeps [`LINE_GROWTH_SLACK`] spare entries in every per-line array.
fn reserve_line_slack(cache: &mut DocumentCache) {
    fn slack<T>(v: &mut Vec<T>) {
        if v.capacity() - v.len() < LINE_GROWTH_SLACK {
            v.reserve_exact(LINE_GROWTH_SLACK);
        }
    }
    slack(&mut cache.line_roles);
    slack(&mut cache.fence_states);
    slack(&mut cache.block_contexts);
    slack(&mut cache.open_block_states);
    slack(&mut cache.callout_states);
}

#[cfg(test)]
mod tests;

//! A persistent, edit-patched cache of the four whole-document Live Preview
//! classification passes (`block_preview::classify_lines`,
//! `live_preview::classify_block_context`, `table_preview::classify_tables`,
//! `callout::classify_callouts`) — see ARCHITECTURE.md's own bullet on
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
//! the document — see `fenwick.rs`'s own doc comment for why that matters.
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

use std::ops::Range;

use super::block_preview::{self, LineRole};
use super::callout::{self, Callout, CalloutState};
use super::columns::{self, Columns, Placed};
use super::fenwick::{Fenwick, LINE_GROWTH_SLACK};
use super::images::ImageSizes;
use super::live_preview::{self, LineContext, OpenBlock};
use super::table_preview::{self, Table, is_pipe_row};

/// A tree-sitter-`InputEdit`-style description of what just changed in the
/// buffer: the byte range `start_byte..old_end_byte` in the *old* text was
/// replaced by `new_end_byte - start_byte` bytes of new content. Every
/// mutating call site (`editor.rs`, `input_handler.rs`) derives this from
/// `cursor()`/`selection()`/`text().len()` reads taken immediately before
/// and after the actual buffer mutation — all O(1), no scan — rather than
/// `EditorBuffer` reporting it for plain edits. Undo/redo report theirs
/// directly (`EditorBuffer::undo`/`redo`): an undo entry is a delta, so the
/// replaced range is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TextEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
}

/// Where an edit falls in line terms — shared bookkeeping every per-
/// classifier patch function needs, computed once in [`DocumentCache::
/// record_edit`] rather than once per classifier.
struct EditSpan {
    /// The old line index `edit.start_byte` falls on (also its new index:
    /// nothing before the edit moves).
    start_line: usize,
    /// How many old lines the edit's old byte range spanned across (0 if
    /// it never crossed a newline at all).
    lines_removed: usize,
    /// How many newlines the edit's new text contains.
    lines_added: usize,
}

impl EditSpan {
    /// The last line (in the *new* numbering) whose text the edit itself
    /// changed. Reconvergence in the patch walks is only trustworthy past
    /// this line.
    fn last_new_line(&self) -> usize {
        self.start_line + self.lines_added
    }

    fn line_delta(&self) -> isize {
        self.lines_added as isize - self.lines_removed as isize
    }

    /// Where per-line arrays gain or lose entries so that everything after
    /// the edit lines up with its new line numbers: the entries before
    /// this point cover lines the walk rewrites anyway, the ones from it
    /// on are the untouched old tail.
    fn splice_at(&self) -> usize {
        self.start_line + self.lines_removed.min(self.lines_added) + 1
    }
}

/// What [`DocumentCache::record_edit`] changed, for `RowHeightCache` to
/// mirror: resize at `splice_at` by `line_delta` lines, then re-derive
/// `touched`.
pub(super) struct EditPatch {
    pub touched: Range<usize>,
    pub splice_at: usize,
    pub line_delta: isize,
}

impl EditPatch {
    /// The lines `touched` covered before the edit: later lines shifted by
    /// `line_delta`, so the old extent ends that much earlier. What a
    /// per-line table splices its replacement over.
    pub(super) fn old_touched(&self) -> Range<usize> {
        let end = self.touched.end as isize - self.line_delta;
        assert!(end >= self.touched.start as isize);
        self.touched.start..end as usize
    }
}

/// What `patch_tables` must learn from the *old* table layout before the
/// edit's line shift destroys it.
struct TablePre {
    /// Whether any old line the edit replaced belonged to a table.
    touched_old_table: bool,
    /// The union of the old line extents of every table that did.
    old_extent: Option<(usize, usize)>,
}

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
    /// counting newlines from the start of the document. See `fenwick.rs`.
    /// Shared across classifiers — there's only one true line layout.
    line_index: Fenwick,
    block_contexts: Vec<LineContext>,
    /// `open_block_states[i]` is the `OpenBlock` *entering* line `i`.
    /// Parallel to `block_contexts`, same role `fence_states` plays for
    /// `line_roles`.
    open_block_states: Vec<OpenBlock>,
    tables: Vec<Table>,
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
    column_markers: Vec<Placed>,
    /// The regions `column_markers` form.
    columns: Vec<Columns>,
    /// The width regions are laid out across. `None` lays every region out
    /// stacked; set by whoever builds the row counts (`RowHeightCache`).
    column_width: Option<f32>,
    /// The region shown as source (Ctrl+/): stacked, its markers visible.
    raw_region: Option<usize>,
    /// Natural sizes of the images the note embeds, read off the main thread.
    images: ImageSizes,
    /// The table shown as raw text (Ctrl+/, or the caret on its delimiter
    /// row) instead of rendered; see `Editor::sync_table_view`.
    raw_table: Option<usize>,
    /// The table cell the caret is in, as `(line, column)`: it shows its text
    /// as typed and is what row counts and painting treat as being edited.
    editing_cell: Option<(usize, usize)>,
    /// Prose is justified: set from the settings each frame.
    justify: bool,
    /// The shares a divider drag is previewing for one region (its index and
    /// the shares), laid out in place of the source's until the drag ends.
    column_preview: Option<(usize, Vec<u8>)>,
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
            callouts: Vec::new(),
            callout_states: Vec::new(),
            callout_scratch: Vec::new(),
            column_markers: Vec::new(),
            columns: Vec::new(),
            column_width: None,
            raw_region: None,
            images: ImageSizes::default(),
            raw_table: None,
            editing_cell: None,
            justify: false,
            column_preview: None,
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

    pub(super) fn column_width(&self) -> Option<f32> {
        self.column_width
    }

    /// Sets the width regions are laid out across; `true` when it changed.
    pub(super) fn set_column_width(&mut self, width: Option<f32>) -> bool {
        std::mem::replace(&mut self.column_width, width) != width
    }

    /// The index of the region `line` is in.
    pub(super) fn region_index_at(&self, line: usize) -> Option<usize> {
        let i = self.columns.partition_point(|r| r.end_line <= line);
        self.columns.get(i).filter(|r| r.start_line <= line)?;
        Some(i)
    }

    pub(super) fn raw_region(&self) -> Option<usize> {
        self.raw_region
    }

    /// Shows region `index` as source; `true` when that changed, which changes
    /// every row count in it.
    pub(super) fn set_raw_region(&mut self, index: Option<usize>) -> bool {
        std::mem::replace(&mut self.raw_region, index) != index
    }

    /// The region `line` is in and its split, when the region is laid out side
    /// by side (a width is set, every column fits and it is not shown as
    /// source). `None` otherwise.
    pub(super) fn side_by_side(&self, line: usize) -> Option<(Columns, columns::Split)> {
        let width = self.column_width?;
        let index = self
            .region_index_at(line)
            .filter(|&i| Some(i) != self.raw_region)?;
        let mut region = self.columns[index];
        if let Some((_, shares)) = self
            .column_preview
            .as_ref()
            .filter(|(i, s)| *i == index && s.len() == region.count)
        {
            region = region.with_shares(shares);
        }
        let split = match region.kind {
            columns::Kind::Aside => columns::aside_split(width, region.floats())?,
            columns::Kind::Columns => columns::split(width, &region.percent[..region.count])?,
        };
        Some((region, split))
    }

    /// The region whose shares a divider drag is previewing.
    pub(super) fn column_preview_region(&self) -> Option<usize> {
        self.column_preview.as_ref().map(|(index, _)| *index)
    }

    /// Sets the shares a divider drag previews; `true` when that changed what
    /// is laid out, which changes the region's height.
    pub(super) fn set_column_preview(&mut self, preview: Option<(usize, Vec<u8>)>) -> bool {
        std::mem::replace(&mut self.column_preview, preview.clone()) != preview
    }

    /// Whether `line` is the open or close marker of a region shown as
    /// columns or stacked; none of a region shown as source.
    pub(super) fn is_column_frame(&self, line: usize) -> bool {
        let raw = self.raw_region.and_then(|i| self.columns.get(i));
        columns::is_frame_line(&self.columns, line)
            && !raw.is_some_and(|r| (r.start_line..r.end_line).contains(&line))
    }

    /// Folds or unfolds callout `index` for this session.
    pub(super) fn set_folded(&mut self, index: usize, folded: bool) {
        if let Some(c) = self.callouts.get_mut(index).filter(|c| c.foldable) {
            c.folded = folded;
        }
    }

    pub(super) fn raw_table(&self) -> Option<usize> {
        self.raw_table
    }

    pub(super) fn editing_cell(&self) -> Option<(usize, usize)> {
        self.editing_cell
    }

    /// Sets the view state of the tables; `true` when the raw one changed, which
    /// changes every row count in two tables.
    pub(super) fn set_table_view(
        &mut self,
        raw_table: Option<usize>,
        editing_cell: Option<(usize, usize)>,
    ) -> bool {
        self.editing_cell = editing_cell;
        std::mem::replace(&mut self.raw_table, raw_table) != raw_table
    }

    /// Whether prose is justified (a paint-only setting: no row count changes).
    pub(super) fn justify(&self) -> bool {
        self.justify
    }

    pub(super) fn set_justify(&mut self, on: bool) {
        self.justify = on;
    }

    pub(super) fn images(&self) -> &ImageSizes {
        &self.images
    }

    pub(super) fn images_mut(&mut self) -> &mut ImageSizes {
        &mut self.images
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
        self.line_index = Fenwick::from_vec(strides);
        // Reads lines through the index, so only now.
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

        let touched = [minimal, r1, r2, r3, r4, r5]
            .into_iter()
            .reduce(union_ranges)
            .unwrap();
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
            states.push(block_preview::pack_fence(state));
            let (role, next) = block_preview::step(state, line);
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
            state = callout::step(state, line, i, &mut callouts).1;
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
            let (ctx, next) = live_preview::block_context_step(open, line);
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
        self.tables = table_preview::classify_table_list_from(lines_of(text, strides));
    }

    /// Every line's start byte, plus the document's length as a final entry
    /// (`line_count() + 1` values), in one O(n) sweep: line `i` is
    /// `starts[i]..starts[i + 1] - 1` (without its `\n`), except the last
    /// line, which runs to `starts[line_count()]`. Far cheaper than scanning
    /// the text for newlines or calling [`Self::line_start_byte`] per line.
    pub(super) fn line_start_offsets(&self) -> Vec<usize> {
        self.line_index
            .prefix_sums()
            .into_iter()
            .map(|offset| offset as usize)
            .collect()
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

    /// The table whose lines include `line`, found by line number (the
    /// `table_for_line` indices are stale for tables after a dropped one).
    fn table_containing(&self, line: usize) -> Option<&Table> {
        let i = self.tables.partition_point(|t| t.end_line <= line);
        self.tables.get(i).filter(|t| t.start_line <= line)
    }

    /// Reads, from the *old* layout, which tables the edit's replaced lines
    /// belonged to. Must run before [`Self::resize_lines`].
    fn table_pre(&self, span: &EditSpan) -> TablePre {
        let (first, end) = self.tables_touching(span);
        let touched = &self.tables[first..end];
        TablePre {
            touched_old_table: !touched.is_empty(),
            old_extent: touched
                .first()
                .zip(touched.last())
                .map(|(a, b)| (a.start_line.min(b.start_line), a.end_line.max(b.end_line))),
        }
    }

    /// The `tables[first..end]` that contain a line the edit replaced (old
    /// numbering: call before [`Self::resize_lines`]).
    fn tables_touching(&self, span: &EditSpan) -> (usize, usize) {
        let old_last = (span.start_line + span.lines_removed).min(self.line_roles.len() - 1);
        let first = self
            .tables
            .partition_point(|t| t.end_line <= span.start_line);
        let end = self.tables.partition_point(|t| t.start_line <= old_last);
        (first, end.max(first))
    }

    /// The real incremental path for `table_preview`: unlike the other
    /// three classifiers, a table's column widths are an aggregate over
    /// its whole row range, so there's no meaningful per-line "entering
    /// state" to resume from — instead, this rescans just the bounded
    /// window a table could possibly have been affected in.
    ///
    /// If none of the directly-edited lines used to belong to a table and
    /// none of them are pipe-rows in the *new* text either, nothing here
    /// could possibly have changed — a genuine O(1) no-op for the common
    /// case (editing a paragraph nowhere near a table). Otherwise, the
    /// window is the union of: the edited range itself, the full line
    /// span of any old table any edited line belonged to (a change to one
    /// row, even far from this table's header, can invalidate the whole
    /// table), and the maximal run of pipe-row lines (in the new text)
    /// containing that — a table can only ever start or end at a
    /// non-pipe-row boundary, so within one such run there's at most one
    /// table either way (`table_preview::classify_tables`'s own row-
    /// consumption loop never starts a second one mid-run).
    ///
    /// `table_preview::classify_tables` (the existing, already-correct
    /// whole-document function) is reused directly on just that window's
    /// substring, then spliced back in: tables entirely before/after the
    /// window are kept, with only `table_for_line` entries actually inside
    /// the window or belonging to a kept-after table whose index shifted
    /// touched — bounded by the window's own size plus the total number of
    /// tables and the line spans of ones kept, not by document length.
    fn patch_tables(&mut self, text: &str, span: &EditSpan, pre: &TablePre) -> Range<usize> {
        let line_count = self.line_roles.len();
        let start_line = span.start_line;
        let last_touched_line = span.last_new_line().min(line_count.saturating_sub(1));

        let touched_new_pipe_row =
            (start_line..=last_touched_line).any(|i| is_pipe_row(self.line_text_at(text, i)));
        // The line above the edit may be a header whose delimiter row (not
        // necessarily containing a `|`) the edit just created.
        let header_above = start_line > 0 && is_pipe_row(self.line_text_at(text, start_line - 1));
        if !pre.touched_old_table && !touched_new_pipe_row && !header_above {
            // No table's lines changed (byte ranges are derived, see
            // `table_byte_range`), so only the raw-text-changed span is dirty.
            return start_line..(last_touched_line + 1);
        }

        let mut window_start = start_line;
        let mut window_end = last_touched_line + 1;
        if let Some((table_start, table_end)) = pre.old_extent {
            // `resize_lines` already dropped those tables; their extents
            // are in the old numbering, and the end (always at or past the
            // edit) moves with it.
            window_start = window_start.min(table_start);
            // May go negative for a table that ended inside a deleted
            // range; the touched end then wins the `max`.
            window_end = window_end.max((table_end as isize + span.line_delta()).max(0) as usize);
        }
        // Grow to a fixpoint: a run of pipe rows, plus any existing table
        // touching either edge. A table's delimiter row needn't contain a
        // `|`, so walking pipe rows alone can stop between a table's header
        // and its body.
        loop {
            let before = (window_start, window_end);
            while window_start > 0 && is_pipe_row(self.line_text_at(text, window_start - 1)) {
                window_start -= 1;
            }
            while window_end < line_count && is_pipe_row(self.line_text_at(text, window_end)) {
                window_end += 1;
            }
            // A pipe row ending the window may be a header whose delimiter
            // row (which needn't contain a `|`) is the next line.
            if window_end < line_count && is_pipe_row(self.line_text_at(text, window_end - 1)) {
                window_end += 1;
            }
            let edges = [
                Some(window_start),
                window_start.checked_sub(1),
                Some(window_end - 1),
            ]
            .into_iter()
            .flatten()
            .chain((window_end < line_count).then_some(window_end));
            for line in edges {
                if let Some(t) = self.table_containing(line) {
                    window_start = window_start.min(t.start_line);
                    window_end = window_end.max(t.end_line);
                }
            }
            if (window_start, window_end) == before {
                break;
            }
        }

        let window_start_byte = self.line_index.prefix_sum(window_start) as usize;
        let window_end_byte = self.line_end_byte(window_end - 1);
        let window_text = &text[window_start_byte..window_end_byte];
        let local_tables = table_preview::classify_table_list(window_text);

        let prefix_table_count = self.tables.partition_point(|t| t.end_line <= window_start);
        let suffix_start_idx = self.tables.partition_point(|t| t.start_line < window_end);

        // In place: untouched tables before and after the window are not
        // copied (cloning each one's columns per keystroke cost ~0.4ms).
        self.tables.splice(
            prefix_table_count..suffix_start_idx,
            local_tables.into_iter().map(|t| Table {
                start_line: t.start_line + window_start,
                end_line: t.end_line + window_start,
                columns: t.columns,
            }),
        );
        window_start..window_end
    }

    /// The incremental path for `columns`: rescans the markers in the window
    /// of lines whose text or code-ness changed (`roles` is what
    /// `patch_block_preview` re-classified) plus one neighbour each side, since
    /// a marker needs blank lines around it; the markers outside it are kept
    /// (the ones after it move by the line delta) and the regions re-paired from
    /// all of them, which is cheap because markers are few. A region's height
    /// follows its content, so the returned range covers every region (before
    /// or after the edit) that touches the window, not just the window.
    fn patch_columns(&mut self, text: &str, span: &EditSpan, roles: &Range<usize>) -> Range<usize> {
        let line_count = self.line_roles.len();
        let delta = span.line_delta();
        let lo = span.start_line.min(roles.start).saturating_sub(1);
        let hi = (span.last_new_line() + 1)
            .max(roles.end)
            .saturating_add(1)
            .min(line_count);
        let old_hi = (hi as isize - delta) as usize;
        assert!(
            lo <= hi && lo <= old_hi,
            "window {lo}..{hi}, old end {old_hi}"
        );

        let old_extent = extent_touching(&self.columns, lo, old_hi).map(|(start, end)| {
            let to_new = |x: usize| match x {
                x if x < lo => x,
                x if x >= old_hi => shift_line(x, delta),
                _ => lo,
            };
            (to_new(start), to_new(end))
        });

        let before_blank = lo == 0 || columns_blank(self.line_text_at(text, lo - 1));
        let after_blank = hi == line_count || columns_blank(self.line_text_at(text, hi));
        let lines = (lo..hi).map(|i| {
            (
                self.line_text_at(text, i),
                !matches!(self.line_roles[i], LineRole::Plain),
            )
        });
        let fresh = columns::place_markers(lo, lines, before_blank, after_blank);

        let first = self.column_markers.partition_point(|m| m.line < lo);
        let end = self.column_markers.partition_point(|m| m.line < old_hi);
        // Typing on a line that is no marker and was none: the markers and so
        // the regions are as they were, which is nearly every keystroke.
        let no_markers = first == end && fresh.is_empty();
        if !(no_markers && delta == 0) {
            let fresh_len = fresh.len();
            self.column_markers.splice(first..end, fresh);
            for m in &mut self.column_markers[first + fresh_len..] {
                m.line = shift_line(m.line, delta);
            }
            self.columns = columns::derive(&self.column_markers, line_count);
        }
        self.cap_tails(text);

        let new_extent = extent_touching(&self.columns, lo, hi);
        // The neighbours are only dirty when a marker came or went (their
        // isolation changed); otherwise just the lines the edit rewrote.
        let mut touched = if no_markers {
            span.start_line..span.last_new_line() + 1
        } else {
            lo..hi
        };
        for (start, end) in old_extent.into_iter().chain(new_extent) {
            touched = union_ranges(touched, start..end);
        }
        touched.start..touched.end.min(line_count)
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

    /// Resolves `edit`'s [`EditSpan`] against the *old* `line_index`, or
    /// `None` only for an empty cache (nothing to patch). Edits that change
    /// the line count are handled by [`Self::resize_lines`].
    fn edit_span(&self, text: &str, edit: TextEdit) -> Option<EditSpan> {
        if self.line_roles.is_empty() {
            return None;
        }
        let start_line = self.line_at(edit.start_byte);
        let end_line = self.line_at(edit.old_end_byte);
        let lines_removed = end_line.saturating_sub(start_line);
        let lines_added = text[edit.start_byte..edit.new_end_byte]
            .matches('\n')
            .count();
        Some(EditSpan {
            start_line,
            lines_removed,
            lines_added,
        })
    }

    /// The real incremental path for `block_preview`: resumes from the
    /// edited line's cached entering fence state, walks forward over the
    /// *new* text (one line at a time, via [`lines_from`] — never
    /// materializing the whole document's lines, see this module's own doc
    /// comment) re-running [`block_preview::step`], and stops the moment a
    /// freshly computed exit state matches what was already cached for the
    /// next line — everything after that point is provably identical to
    /// before (same state in, same unchanged text, same result), so it's
    /// reused as-is rather than recomputed.
    fn patch_block_preview(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.line_roles.len();
        let start_line = span.start_line;
        // The last *old* line index the edit actually replaced — every new
        // line up to and including this one is guaranteed genuinely new
        // content (part of the inserted text, however the state machine
        // happens to classify it), even if a state coincidentally matches
        // the old cached one somewhere in between. Only once the walk
        // passes this point is a matching state actually trustworthy
        // evidence that "the rest is identical to before" — skipping this
        // check and reconverging on a merely coincidental state match is
        // exactly what an earlier version of this function got wrong on a
        // multi-line, same-line-count-delta edit, caught by this module's
        // own randomized property test, not reasoned out in advance.
        let last_touched_line = span.last_new_line();

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = block_preview::unpack_fence(self.fence_states[start_line]);
        for (i, (line, line_len, has_next_line)) in (start_line..).zip(lines_from(text, start_byte))
        {
            let (role, next_state) = block_preview::step(state, line);
            self.line_roles[i] = role;

            let old_len = self.line_index.prefix_sum(i + 1) - self.line_index.prefix_sum(i);
            let new_len = line_len as i64 + if has_next_line { 1 } else { 0 };
            self.line_index.add_at(i, new_len - old_len);

            let has_next = i + 1 < old_line_count;
            let packed_next = block_preview::pack_fence(next_state);
            let reconverged =
                has_next && i >= last_touched_line && self.fence_states[i + 1] == packed_next;
            if has_next {
                self.fence_states[i + 1] = packed_next;
            }
            if reconverged || !has_next {
                return start_line..(i + 1);
            }
            state = next_state;
        }
        start_line..old_line_count
    }

    /// The real incremental path for `callout`: the same resumable-walk
    /// shape as `patch_block_preview`. The walk writes the callouts it
    /// re-derives into `callout_scratch` (seeded with the one callout that
    /// may be open across the edit), and they are spliced over the old
    /// callouts the walked lines covered. A callout's state holds no index,
    /// so an edit that adds or removes a callout still reconverges where
    /// the line states match again. The callout open at that point had its
    /// `end_line` cut off by the walk, so it takes the old callout's end.
    /// Callouts after the splice only move when the line count changed.
    fn patch_callouts(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.line_roles.len();
        let start_line = span.start_line;
        let last_touched_line = span.last_new_line();
        let delta = span.line_delta();

        let entering = CalloutState::unpack(self.callout_states[start_line]);
        let before_edit = self.callouts.partition_point(|c| c.start_line < start_line);
        let mut fresh = std::mem::take(&mut self.callout_scratch);
        fresh.clear();
        let mut replace_from = before_edit;
        if entering.callout_open {
            // The open callout is the last one starting before the edit.
            replace_from = before_edit - 1;
            fresh.push(self.callouts[replace_from].clone());
        }

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = entering;
        let mut last_walked = old_line_count - 1;
        let mut reconverged = false;
        let mut open_at_stop = false;
        for (i, (line, ..)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (_, next_state) = callout::step(state, line, i, &mut fresh);
            let packed_next = next_state.pack();
            last_walked = i;
            if i + 1 >= old_line_count {
                break;
            }
            reconverged = i >= last_touched_line && self.callout_states[i + 1] == packed_next;
            self.callout_states[i + 1] = packed_next;
            if reconverged {
                open_at_stop = next_state.callout_open;
                break;
            }
            state = next_state;
        }

        let replace_to = if reconverged {
            // Old callouts starting before the old line the walk stopped at
            // were re-derived; the rest are the untouched tail.
            let old_next = shift_line(last_walked + 1, -delta);
            let to = self.callouts.partition_point(|c| c.start_line < old_next);
            if open_at_stop {
                debug_assert!(
                    to > replace_from,
                    "an open callout must be in the replaced range"
                );
                let old_end = self.callouts[to - 1].end_line;
                if let Some(open) = fresh.last_mut() {
                    open.end_line = shift_line(old_end, delta);
                }
            }
            to
        } else {
            self.callouts.len()
        };
        let fresh_len = fresh.len();
        carry_fold_state(
            &mut fresh,
            &self.callouts[replace_from..replace_to],
            shift_line(last_touched_line, -delta),
            delta,
        );
        self.callouts
            .splice(replace_from..replace_to, fresh.drain(..));
        if reconverged && delta != 0 {
            for c in &mut self.callouts[replace_from + fresh_len..] {
                c.start_line = shift_line(c.start_line, delta);
                c.end_line = shift_line(c.end_line, delta);
            }
        }
        self.callout_scratch = fresh;
        start_line..(last_walked + 1)
    }

    /// The real incremental path for `live_preview`'s block context: the
    /// same resumable-walk shape as `patch_block_preview` — `OpenBlock`
    /// never references an external index the way a callout's state does,
    /// so (unlike `patch_callouts`) a plain state-equality check at
    /// reconvergence is enough on its own.
    fn patch_block_context(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.block_contexts.len();
        let start_line = span.start_line;
        let last_touched_line = span.last_new_line();

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = self.open_block_states[start_line].clone();
        for (i, (line, ..)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (ctx, next_state) = live_preview::block_context_step(state, line);
            self.block_contexts[i] = ctx;

            let has_next = i + 1 < old_line_count;
            let reconverged =
                has_next && i >= last_touched_line && self.open_block_states[i + 1] == next_state;
            if has_next {
                self.open_block_states[i + 1] = next_state.clone();
            }
            if reconverged || !has_next {
                return start_line..(i + 1);
            }
            state = next_state;
        }
        start_line..old_line_count
    }
}

/// The union of the line extents of the regions that overlap lines `a..b`.
fn extent_touching(regions: &[Columns], a: usize, b: usize) -> Option<(usize, usize)> {
    // Regions are disjoint and sorted, so both ends are sorted too.
    let first = regions.partition_point(|r| r.end_line <= a);
    let end = regions.partition_point(|r| r.start_line < b).max(first);
    let touching = &regions[first..end];
    Some((touching.first()?.start_line, touching.last()?.end_line))
}

fn columns_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The smallest range containing both `a` and `b` — used by
/// [`DocumentCache::record_edit`] to union each classifier's own touched
/// range into one overall "these lines might need their row heights
/// recomputed" range.
fn union_ranges(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
}

/// The number of lines `text.split('\n')` yields; the oracle for
/// [`line_strides`].
#[cfg(test)]
fn count_lines(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .map(|&b| usize::from(b == b'\n'))
        .sum::<usize>()
        + 1
}

/// Each line's byte stride (`len + 1`, except the last line, which has no
/// trailing `\n`), with room for [`LINE_GROWTH_SLACK`] more lines. The one
/// scan for newlines in a cold build.
pub(super) fn line_strides(text: &str) -> Vec<i64> {
    // A guess at the line count (the stress fixture averages 37 bytes per
    // line): counting first costs as much as the scan, and growing the
    // vector once or twice is cheaper than that.
    let mut strides = Vec::with_capacity(text.len() / 32 + LINE_GROWTH_SLACK);
    let mut previous = 0;
    for_each_newline(text.as_bytes(), |newline| {
        strides.push((newline + 1 - previous) as i64);
        previous = newline + 1;
    });
    strides.push((text.len() - previous) as i64);
    strides
}

/// Calls `visit` with the byte offset of every `\n`, in order. Short lines
/// make a `memchr` call per line cost more than the scan itself, so the text
/// is scanned a block at a time and only the set bits are visited.
fn for_each_newline(bytes: &[u8], visit: impl FnMut(usize)) {
    #[cfg(target_arch = "x86_64")]
    return for_each_newline_sse2(bytes, visit);
    #[cfg(not(target_arch = "x86_64"))]
    for_each_newline_swar(bytes, visit);
}

/// 16 bytes per step with SSE2, which every x86_64 CPU has.
#[cfg(target_arch = "x86_64")]
fn for_each_newline_sse2(bytes: &[u8], mut visit: impl FnMut(usize)) {
    use std::arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    };
    let (chunks, remainder) = bytes.as_chunks::<16>();
    let mut base = 0;
    // SAFETY: SSE2 is part of the x86_64 baseline; each load reads exactly
    // the 16 bytes of `chunk` (unaligned loads are allowed).
    let newline = unsafe { _mm_set1_epi8(b'\n' as i8) };
    for chunk in chunks {
        let mut mask = unsafe {
            let block = _mm_loadu_si128(chunk.as_ptr().cast::<__m128i>());
            _mm_movemask_epi8(_mm_cmpeq_epi8(block, newline)) as u32
        };
        while mask != 0 {
            visit(base + mask.trailing_zeros() as usize);
            mask &= mask - 1;
        }
        base += 16;
    }
    for (i, &byte) in remainder.iter().enumerate() {
        if byte == b'\n' {
            visit(base + i);
        }
    }
}

/// Eight bytes per step in plain integer arithmetic, for targets without
/// the SSE2 path (and as its oracle in the tests).
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
fn for_each_newline_swar(bytes: &[u8], mut visit: impl FnMut(usize)) {
    const LOW_7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const NEWLINES: u64 = 0x0a0a_0a0a_0a0a_0a0a;
    let (chunks, remainder) = bytes.as_chunks::<8>();
    let mut base = 0;
    for &chunk in chunks {
        let x = u64::from_le_bytes(chunk) ^ NEWLINES;
        // The high bit of each byte that was `\n` (so is zero in `x`), and
        // no other bit: the exact zero-byte test, free of false positives.
        let mut zero_bytes = !(((x & LOW_7) + LOW_7) | x | LOW_7);
        while zero_bytes != 0 {
            visit(base + (zero_bytes.trailing_zeros() / 8) as usize);
            zero_bytes &= zero_bytes - 1;
        }
        base += 8;
    }
    for (i, &byte) in remainder.iter().enumerate() {
        if byte == b'\n' {
            visit(base + i);
        }
    }
}

/// The lines `strides` (from [`line_strides`] of `text`) describe, without
/// scanning for newlines again.
fn lines_of<'a>(text: &'a str, strides: &'a [i64]) -> impl Iterator<Item = &'a str> + 'a {
    let last = strides.len().saturating_sub(1);
    let mut start = 0;
    strides.iter().enumerate().map(move |(i, &stride)| {
        let stride = stride as usize;
        let end = start + if i < last { stride - 1 } else { stride };
        let line = &text[start..end];
        start += stride;
        line
    })
}

/// `line` moved by `delta` lines.
/// A callout the user folded or unfolded keeps that state when an edit
/// re-derives it, matched by its header line (the walk restarts every callout
/// it touches from the marker's own state). `old_last` is the last touched
/// line in the old numbering; lines after it moved by `delta`.
fn carry_fold_state(fresh: &mut [Callout], old: &[Callout], old_last: usize, delta: isize) {
    let moved = |line: usize| {
        if line <= old_last {
            line
        } else {
            shift_line(line, delta)
        }
    };
    for c in fresh {
        let kept = old
            .iter()
            .find(|old| old.foldable && c.foldable && moved(old.start_line) == c.start_line);
        if let Some(old) = kept {
            c.folded = old.folded;
        }
    }
}

fn shift_line(line: usize, delta: isize) -> usize {
    let shifted = line as isize + delta;
    debug_assert!(shifted >= 0, "line {line} shifted by {delta}");
    shifted.max(0) as usize
}

/// Inserts `delta` copies of the entry before `at` (`delta > 0`) or removes
/// `-delta` entries starting at `at` (`delta < 0`). Grows in fixed chunks:
/// see [`LINE_GROWTH_SLACK`].
fn resize_vec<T: Clone>(v: &mut Vec<T>, at: usize, delta: isize) {
    if delta > 0 {
        let k = delta as usize;
        let fill = v[at - 1].clone();
        if v.capacity() - v.len() < k {
            v.reserve_exact(k.max(LINE_GROWTH_SLACK));
        }
        v.splice(at..at, std::iter::repeat_n(fill, k));
    } else {
        v.drain(at..at + delta.unsigned_abs());
    }
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

/// Yields `(line_text, line_len, has_next_line)` starting at `start_byte`
/// (which must be a real line-start byte offset in `text`), walking
/// forward via a running cursor and `text[cursor..].find('\n')` rather
/// than ever materializing the whole document's lines — the technique
/// every patch function above relies on to stay bounded by how far it
/// actually walks, not by the document's total length. The same bounded-
/// scan idea `buffer.rs`'s own `line_start`/`line_end` already use, just
/// walking forward instead of from a single point.
fn lines_from(text: &str, start_byte: usize) -> impl Iterator<Item = (&str, usize, bool)> {
    let mut cursor = start_byte;
    std::iter::from_fn(move || {
        if cursor > text.len() {
            return None;
        }
        let rest = &text[cursor..];
        let line_len = rest.find('\n').unwrap_or(rest.len());
        let line = &rest[..line_len];
        let has_next_line = cursor + line_len < text.len();
        cursor += line_len + 1;
        Some((line, line_len, has_next_line))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::text_editor::test_support::apply_edit;

    /// The property `patch_block_preview` actually relies on: after any
    /// sequence of edits (including ones that change the document's line
    /// count, forcing the fallback path), the incrementally-maintained
    /// `line_roles` must equal calling `classify_lines` fresh on the
    /// resulting text — the "oracle" `block_preview.rs`'s own module doc
    /// now describes it as.
    #[test]
    fn incremental_block_preview_matches_the_oracle_after_a_hand_picked_sequence_of_edits() {
        let mut text = String::from(
            "intro\n```\ncode line one\ncode line two\n```\nplain again\n> quote\n- item",
        );
        let mut cache = DocumentCache::new(&text);
        assert_eq!(
            cache.line_roles(),
            block_preview::classify_lines(&text).as_slice()
        );

        // (start, delete_len, insert) — a mix of same-line-count edits
        // (exercising `patch_block_preview`'s real incremental path) and
        // line-count-changing ones (exercising its fallback).
        let edits: &[(usize, usize, &str)] = &[
            (0, 0, "x"),                // insert at the very start
            (5, 0, "!"),                // insert mid-line, no newline
            (9, 0, "\n"),               // insert a bare newline
            (0, 1, ""),                 // delete one char at the start
            (0, 0, "```\n"),            // insert a new fence delimiter line
            (0, 0, "~~~\nmore\n~~~\n"), // insert a whole tilde-fenced block
        ];

        for &(start, delete_len, insert) in edits {
            let old_end = start + delete_len;
            let (new_text, edit) = apply_edit(&text, start, old_end, insert);
            cache.record_edit(&new_text, edit);
            let expected = block_preview::classify_lines(&new_text);
            assert_eq!(
                cache.line_roles(),
                expected.as_slice(),
                "after editing {:?}",
                (start, old_end, insert)
            );
            text = new_text;
        }
    }

    /// Hand-picked edge cases `patch_tables` needs to get right that a
    /// short random run might not stumble into: merging two previously
    /// separate tables by deleting the blank line between them, splitting
    /// one table into two by inserting a blank line in the middle,
    /// invalidating a table entirely by corrupting its delimiter row, and
    /// an edit far from any table that must stay a complete no-op.
    #[test]
    fn incremental_tables_handles_merge_split_and_invalidate() {
        let mut text = String::from(
            "before\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n| c | d |\n| --- | --- |\n| 3 | 4 |\n\nafter",
        );
        let mut cache = DocumentCache::new(&text);
        assert_eq!(
            cache.tables().len(),
            2,
            "sanity: two separate tables to start"
        );

        // Delete the blank line between the two tables — `classify_tables`
        // would now see one unbroken pipe-row run, but since "| 1 | 2 |"
        // isn't a valid delimiter row for a 2-column header, this merges
        // into a *single* table (the first one's body absorbing
        // everything through the second table's own rows) rather than two.
        let blank_line_start = text.find("\n\n| c").unwrap() + 1;
        let (new_text, edit) = apply_edit(&text, blank_line_start, blank_line_start + 1, "");
        cache.record_edit(&new_text, edit);
        let (expected_for_line, expected_tables) = table_preview::classify_tables(&new_text);
        assert_eq!(table_for_line_of(&cache), expected_for_line);
        assert_eq!(cache.tables(), expected_tables.as_slice());
        assert_eq!(cache.tables().len(), 1, "the two tables merged into one");
        text = new_text;

        // Corrupt the merged table's own delimiter row so it's no longer
        // valid — the merged table is destroyed, though (per
        // `classify_tables`'s own forward-scanning algorithm) a *smaller*
        // table re-forms further down, where "| c | d |" / the second,
        // still-intact "| --- | --- |" happen to form a valid header/
        // delimiter pair of their own. The real property under test is
        // still just "matches the oracle," not a specific table count —
        // this was caught by that equality check alone, not an assumption
        // written into the test, when an earlier version of this test
        // wrongly assumed the table count would drop to zero.
        let delim_start = text.find("| --- | --- |").unwrap();
        let (new_text, edit) = apply_edit(&text, delim_start, delim_start + 1, "x");
        cache.record_edit(&new_text, edit);
        let (expected_for_line, expected_tables) = table_preview::classify_tables(&new_text);
        assert_eq!(table_for_line_of(&cache), expected_for_line);
        assert_eq!(cache.tables(), expected_tables.as_slice());
        text = new_text;

        // An edit far from anything table-related (the leading "before")
        // must be a complete no-op for this classifier.
        let (new_text, edit) = apply_edit(&text, 0, 0, "BEFORE ");
        cache.record_edit(&new_text, edit);
        let (expected_for_line, expected_tables) = table_preview::classify_tables(&new_text);
        assert_eq!(table_for_line_of(&cache), expected_for_line);
        assert_eq!(cache.tables(), expected_tables.as_slice());
    }

    /// What the removed per-line lookup arrays held, derived from `tables`
    /// and `callouts`: compared against the whole-document oracles.
    fn table_for_line_of(cache: &DocumentCache) -> Vec<Option<usize>> {
        (0..cache.line_count())
            .map(|i| table_preview::table_index_at(cache.tables(), i))
            .collect()
    }

    fn callout_for_line_of(cache: &DocumentCache) -> Vec<Option<usize>> {
        (0..cache.line_count())
            .map(|i| callout::callout_index_at(cache.callouts(), i))
            .collect()
    }

    /// Every field of `cache`, including the entering-state arrays and the
    /// byte-offset index no public accessor exposes, must equal a cache
    /// built fresh from `text`: a stale entry there could survive until a
    /// later edit resumes from it.
    fn assert_matches_fresh_cache(cache: &DocumentCache, text: &str, context: &str) {
        let fresh = DocumentCache::new(text);
        assert_eq!(cache.line_roles, fresh.line_roles, "line_roles {context}");
        assert_eq!(
            cache.fence_states, fresh.fence_states,
            "fence_states {context}"
        );
        assert_eq!(
            cache.block_contexts, fresh.block_contexts,
            "contexts {context}"
        );
        assert_eq!(
            cache.open_block_states, fresh.open_block_states,
            "open_block_states {context}"
        );
        assert_eq!(cache.tables, fresh.tables, "tables {context}");
        assert_eq!(cache.callouts, fresh.callouts, "callouts {context}");
        assert_eq!(
            cache.column_markers, fresh.column_markers,
            "column_markers {context}"
        );
        assert_eq!(cache.columns, fresh.columns, "columns {context}");
        assert_eq!(
            cache.callout_states, fresh.callout_states,
            "callout_states {context}"
        );
        assert_eq!(
            cache.line_index.len(),
            fresh.line_index.len(),
            "line_index len {context}"
        );
        for i in 0..=fresh.line_index.len() {
            assert_eq!(
                cache.line_index.prefix_sum(i),
                fresh.line_index.prefix_sum(i),
                "line_index prefix {i} {context}"
            );
        }
    }

    /// Same property, exercised over many pseudo-random edits — deliberate
    /// coverage of positions/fragments a hand-picked sequence wouldn't
    /// think to try, same "fuzz the invariant against the oracle" approach
    /// `fenwick.rs`'s own random test uses. Covers both `line_roles` and
    /// `callout_for_line`/`callouts` together, since every edit exercises
    /// both patch functions at once via the same `record_edit` call.
    #[test]
    fn incremental_classification_matches_the_oracle_across_many_randomized_edits() {
        for run in 0..25u64 {
            run_randomized_edits(
                0xC0FF_EE15_EA5E_D00Du64 ^ run.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            );
        }
    }

    /// A callout the user folded or unfolded keeps that state through edits in
    /// its body or header and through edits above it that move its lines.
    #[test]
    fn fold_state_survives_edits_that_rederive_the_callout() {
        let text = "intro\n\n> [!note]- Title\n> body\n\n> [!tip]+ Open\n> body\n";
        let mut cache = DocumentCache::new(text);
        assert_eq!(
            cache
                .callouts()
                .iter()
                .map(|c| c.folded)
                .collect::<Vec<_>>(),
            [true, false]
        );
        cache.set_folded(0, false);
        cache.set_folded(1, true);
        let state = |cache: &DocumentCache| {
            cache
                .callouts()
                .iter()
                .map(|c| c.folded)
                .collect::<Vec<_>>()
        };

        let in_body = text.find("body").unwrap();
        let (typed, edit) = apply_edit(text, in_body, in_body, "x");
        cache.record_edit(&typed, edit).unwrap();
        assert_eq!(state(&cache), [false, true], "typing in a body");

        let in_title = typed.find("Title").unwrap();
        let (retitled, edit) = apply_edit(&typed, in_title, in_title, "New ");
        cache.record_edit(&retitled, edit).unwrap();
        assert_eq!(state(&cache), [false, true], "typing in a header");

        let (entered, edit) = apply_edit(&retitled, 0, 0, "\n\n");
        cache.record_edit(&entered, edit).unwrap();
        assert_eq!(state(&cache), [false, true], "lines inserted above");
        assert_eq!(cache.callouts()[0].start_line, 4);

        cache.set_folded(1, false);
        assert!(!cache.callouts()[1].folded);
    }

    /// A character that removes or creates a callout used to shift every
    /// later callout count, so the walk never reconverged and ran to the end
    /// of the document (14ms on the 320K-line stress fixture).
    #[test]
    fn removing_or_creating_a_callout_does_not_walk_to_the_end_of_the_document() {
        let mut text = String::from("intro\n\n> [!note]\n> body\n\n");
        for i in 0..2000 {
            text.push_str(&format!("> [!tip]\n> body {i}\n\n"));
        }
        let mut cache = DocumentCache::new(&text);
        let callouts = cache.callouts().len();
        let blank = "intro\n\n> [!note]\n> body\n".len();

        // "x" on the blank line lazily continues the quote above, so the
        // next header is no longer a fresh blockquote start.
        let (typed, edit) = apply_edit(&text, blank, blank, "x");
        let patch = cache.record_edit(&typed, edit).unwrap();
        assert_eq!(cache.callouts().len(), callouts - 1);
        assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
        assert_matches_fresh_cache(&cache, &typed, "after removing a callout");

        let (restored, edit) = apply_edit(&typed, blank, blank + 1, "");
        let patch = cache.record_edit(&restored, edit).unwrap();
        assert_eq!(cache.callouts().len(), callouts);
        assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
        assert_matches_fresh_cache(&cache, &restored, "after restoring it");

        // An Enter before every later callout shifts them without re-walking.
        let (entered, edit) = apply_edit(&restored, 0, 0, "\n");
        let patch = cache.record_edit(&entered, edit).unwrap();
        assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
        assert_matches_fresh_cache(&cache, &entered, "after Enter at the top");
    }

    /// Float blocks: whatever the edits do to the markers, the blank lines and
    /// the line count, the incrementally kept regions (their capped text
    /// included) are the ones a fresh scan finds.
    #[test]
    fn float_blocks_stay_as_a_fresh_scan_finds_them_across_random_edits() {
        let mut seed = 0x5eed_f10a7_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        let mut text = String::from(
            "intro\n\n::: float left 30%\n\nfloat\n\n:::\n\npara one\n\npara two\n\n\
             ::: columns\n\na\n\n---\n\nb\n\n:::\n\n::: float\n\nf\n\n:::\n\nlast\n",
        );
        let mut tail = String::new();
        for i in 0..40 {
            tail.push_str(&format!("filler {i}\n\n"));
        }
        text.push_str(&tail);
        let mut cache = DocumentCache::new(&text);
        let fragments = [
            "",
            "x",
            "\n",
            "\n\n",
            "```\n",
            "text\n\n",
            ":::\n",
            "\n:::\n\n",
            "---\n",
            "::: float\n",
            "\n::: float right 20%\n\nf\n\n:::\n\n",
            "::: columns\n",
            "> [!note]\n> a\n\n",
        ];
        for step in 0..600 {
            let start = (next() as usize) % (text.len() + 1);
            let delete_len = (next() as usize) % (text.len() - start).min(12).max(1);
            let fragment = fragments[(next() as usize) % fragments.len()];
            let (new_text, edit) = apply_edit(&text, start, start + delete_len, fragment);
            cache.record_edit(&new_text, edit);
            let fresh = DocumentCache::new(&new_text);
            assert_eq!(
                cache.columns, fresh.columns,
                "step={step} edit=({start},{delete_len},{fragment:?}) text={new_text:?}"
            );
            text = new_text;
        }
    }

    fn run_randomized_edits(mut seed: u64) {
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed
        };

        let mut text = String::from(
            "intro\n```\ncode\n```\nplain\n> q\n- item\n  - nested\n    - deeper\nmore plain text here\n~~~\ntilde\n~~~\nend\n\
             > [!info]\n> body\n\n> [!warning] custom\n> more body\nafter\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\nafter table\n\n\
             ::: columns 60/40\n\nleft\n\n---\n\nright\n\n:::\n",
        );
        let mut cache = DocumentCache::new(&text);

        // ASCII-only, so every random byte offset below is guaranteed a
        // valid char boundary too.
        let fragments = [
            "",
            "x",
            "\n",
            "```\n",
            "~~~\n",
            "a\nb\nc",
            "```rust\ncode\n```",
            " ",
            "> [!note]\n",
            "> [!tip]+ title\n> body\n",
            "\n\n",
            "- item\n",
            "  - nested item\n",
            "    - even deeper\n",
            "> quoted\n",
            "# Heading\n",
            "| x | y |\n",
            "| --- | --- |\n",
            "|",
            " extra |",
            "\n| p | q |\n| --- | --- |\n| 5 | 6 |\n",
            "> [!note]\n> a\n> b\n\n",
            "\n\n\n\n",
            "line one\nline two\n",
            "::: columns\n",
            "\n::: columns 20/30/50\n\n",
            "---\n",
            "\n---\n\n",
            ":::\n",
            "\n:::\n\n",
            "::: columns\n\na\n\n---\n\nb\n\n:::\n",
        ];

        for step in 0..400 {
            let start = (next() as usize) % (text.len() + 1);
            let max_delete = text.len() - start;
            let delete_len = if max_delete == 0 {
                0
            } else {
                (next() as usize) % (max_delete + 1)
            };
            let fragment = fragments[(next() as usize) % fragments.len()];
            let old_end = start + delete_len;
            let (new_text, edit) = apply_edit(&text, start, old_end, fragment);
            cache.record_edit(&new_text, edit);

            let expected_roles = block_preview::classify_lines(&new_text);
            assert_eq!(
                cache.line_roles(),
                expected_roles.as_slice(),
                "line_roles step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );

            let (expected_callout_for_line, expected_callouts) =
                callout::classify_callouts(&new_text);
            assert_eq!(
                callout_for_line_of(&cache),
                expected_callout_for_line,
                "callout_for_line step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );
            assert_eq!(
                cache.callouts(),
                expected_callouts.as_slice(),
                "callouts step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );

            let expected_contexts = live_preview::classify_block_context(&new_text);
            assert_eq!(
                cache.block_contexts(),
                expected_contexts.as_slice(),
                "block_contexts step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );

            let (expected_table_for_line, expected_tables) =
                table_preview::classify_tables(&new_text);
            assert_eq!(
                table_for_line_of(&cache),
                expected_table_for_line,
                "table_for_line step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );
            assert_eq!(
                cache.tables(),
                expected_tables.as_slice(),
                "tables step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );

            assert_matches_fresh_cache(
                &cache,
                &new_text,
                &format!("step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"),
            );

            text = new_text;
        }
    }

    /// Diagnostic, not a correctness test — measures (via `crate::test_
    /// alloc`'s counting allocator, not OS-level RSS) how much of
    /// `DocumentCache::new`'s own allocation cost on the 320K-line stress
    /// fixture is specifically `live_preview::classify_block_context`'s
    /// `OpenBlock::List(Vec<usize>)` tracking — every line inside an open
    /// list clones the whole nesting-depth stack (`open = next.clone()` in
    /// `classify_block_context_from`'s loop), a *separate heap allocation
    /// per line*, not shared via `Rc`, persisted forever in `DocumentCache::
    /// open_block_states`. Added after a real user report of surprisingly
    /// high RSS (189MB baseline -> 3xxMB with the stress fixture open, no
    /// edits made) — see HISTORY.md for the measured numbers this produced
    /// and what they did/didn't explain.
    #[test]
    fn block_context_list_tracking_allocation_cost_on_the_stress_fixture() {
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(text) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let line_count = text.split('\n').count();

        crate::test_alloc::reset();
        let before = crate::test_alloc::snapshot();
        let block_context_results =
            live_preview::classify_block_context_from(&text, OpenBlock::None);
        let list_lines = block_context_results
            .iter()
            .filter(|(_, state)| matches!(state, OpenBlock::List(_)))
            .count();
        let after_block_context = crate::test_alloc::snapshot();
        drop(block_context_results);

        crate::test_alloc::reset();
        let before_full = crate::test_alloc::snapshot();
        let cache = DocumentCache::new(&text);
        let after_full_retained = crate::test_alloc::snapshot();
        drop(cache);
        let after_full_dropped = crate::test_alloc::snapshot();

        eprintln!(
            "block_context_list_tracking_allocation_cost_on_the_stress_fixture: \
             {line_count} lines, {list_lines} inside an open list ({:.1}%) | \
             classify_block_context_from alone: {} allocations, peak +{} bytes ({:.1} bytes/line) | \
             full DocumentCache::new (all four classifiers): {} allocations, peak +{} bytes, \
             {} bytes still retained right after construction ({} bytes of transient garbage \
             freed once construction finished, {} bytes correctly freed on drop)",
            100.0 * list_lines as f64 / line_count as f64,
            after_block_context.count - before.count,
            after_block_context.peak - before.peak,
            (after_block_context.peak - before.peak) as f64 / line_count as f64,
            after_full_retained.count - before_full.count,
            after_full_retained.peak - before_full.peak,
            after_full_retained.current - before_full.current,
            (after_full_retained.peak - before_full.peak)
                .saturating_sub(after_full_retained.current - before_full.current),
            after_full_retained.current - after_full_dropped.current,
        );
    }

    const REGION: &str = "top\n\n::: columns 60/40\n\nleft\n\n---\n\nright\n\n:::\n\nbottom";

    #[test]
    fn the_cache_finds_a_region_and_follows_edits_around_it() {
        let mut cache = DocumentCache::new(REGION);
        assert_eq!(cache.columns().len(), 1);
        let r = cache.columns()[0];
        assert_eq!((r.start_line, r.end_line, r.count), (2, 11, 2));

        // Lines added above move it.
        let (text, edit) = apply_edit(REGION, 0, 0, "a\nb\n");
        cache.record_edit(&text, edit);
        assert_eq!(cache.columns()[0].start_line, 4);
        assert_matches_fresh_cache(&cache, &text, "lines above");

        // Breaking the blank line before the separator removes the region.
        let at = text.find("\n---").unwrap();
        let (text, edit) = apply_edit(&text, at, at + 1, "");
        cache.record_edit(&text, edit);
        assert!(cache.columns().is_empty());
        assert_matches_fresh_cache(&cache, &text, "separator joined to text");
    }

    #[test]
    fn an_edit_in_a_column_reports_the_whole_region_as_touched() {
        let mut cache = DocumentCache::new(REGION);
        let at = REGION.find("left").unwrap();
        let (text, edit) = apply_edit(REGION, at, at, "x\ny\n");
        let patch = cache.record_edit(&text, edit).unwrap();
        let r = cache.columns()[0];
        assert!(patch.touched.start <= r.start_line && r.end_line <= patch.touched.end);
        assert_matches_fresh_cache(&cache, &text, "typing in a column");
    }

    #[test]
    fn a_fence_opened_above_hides_the_markers_below() {
        let mut cache = DocumentCache::new(REGION);
        let (text, edit) = apply_edit(REGION, 0, 0, "```\n");
        cache.record_edit(&text, edit);
        assert!(cache.columns().is_empty());
        assert_matches_fresh_cache(&cache, &text, "fence opened");
    }
}

#[cfg(test)]
mod line_stride_tests {
    use super::*;

    #[test]
    fn lines_of_strides_equal_split_on_newline() {
        let texts = [
            "",
            "\n",
            "\n\n",
            "a",
            "a\n",
            "\na",
            "a\n\nb",
            "a\nb\nc\n",
            "é\nü\n\n日本",
        ];
        for text in texts {
            let strides = line_strides(text);
            let lines: Vec<&str> = lines_of(text, &strides).collect();
            let expected: Vec<&str> = text.split('\n').collect();
            assert_eq!(lines, expected, "{text:?}");
            assert_eq!(strides.len(), count_lines(text), "{text:?}");
            assert_eq!(strides.iter().sum::<i64>() as usize, text.len(), "{text:?}");
        }
    }
}

#[cfg(test)]
mod newline_offset_tests {
    use super::{for_each_newline, for_each_newline_swar};

    #[test]
    fn finds_exactly_the_newlines_at_every_alignment_and_length() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // Bytes that stress the zero-byte test: 0x0a itself, its neighbours,
        // 0x8a (high bit set), 0x00 and 0xff.
        let alphabet = [b'\n', b'\n', 0x09, 0x0b, 0x8a, 0x00, 0xff, b'a', b' '];
        for len in 0..140 {
            for _ in 0..200 {
                let bytes: Vec<u8> = (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect();
                let expected: Vec<usize> = bytes
                    .iter()
                    .enumerate()
                    .filter(|&(_, &b)| b == b'\n')
                    .map(|(i, _)| i)
                    .collect();
                let mut got = Vec::new();
                for_each_newline(&bytes, |at| got.push(at));
                assert_eq!(got, expected, "{bytes:?}");
                let mut swar = Vec::new();
                for_each_newline_swar(&bytes, |at| swar.push(at));
                assert_eq!(swar, expected, "swar {bytes:?}");
            }
        }
    }
}

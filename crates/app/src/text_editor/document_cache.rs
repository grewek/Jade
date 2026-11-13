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

use super::block_preview::{self, FenceState, LineRole};
use super::callout::{self, Callout, CalloutState};
use super::fenwick::Fenwick;
use super::live_preview::{self, LineContext, OpenBlock};
use super::table_preview::{self, is_pipe_row, Table};

/// A tree-sitter-`InputEdit`-style description of what just changed in the
/// buffer: the byte range `start_byte..old_end_byte` in the *old* text was
/// replaced by `new_end_byte - start_byte` bytes of new content. Every
/// mutating call site (`editor.rs`, `input_handler.rs`) derives this from
/// `cursor()`/`selection()`/`text().len()` reads taken immediately before
/// and after the actual buffer mutation — all O(1), no scan — rather than
/// `EditorBuffer` itself reporting it, keeping that module's pure/GPUI-free
/// boundary untouched. Undo/redo have no derivable `TextEdit` at all (a
/// `Snapshot` swap could have touched anything) and go through
/// `Editor::invalidate_all` instead.
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
    /// The old line index `edit.start_byte` falls on.
    start_line: usize,
    /// How many old lines the edit's old byte range spanned across (0 if
    /// it never crossed a newline at all).
    lines_removed: usize,
}

/// The persisted result of the four classify_* passes over the buffer's
/// current text — see this module's own doc comment.
pub(super) struct DocumentCache {
    line_roles: Vec<LineRole>,
    /// `fence_states[i]` is the fence state *entering* line `i` (so
    /// `fence_states[0]` is always `None`) — the small per-line value
    /// `patch_block_preview` resumes from and compares against to know
    /// when it can stop walking forward. Parallel to `line_roles`.
    fence_states: Vec<FenceState>,
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
    table_for_line: Vec<Option<usize>>,
    tables: Vec<Table>,
    callout_for_line: Vec<Option<usize>>,
    callouts: Vec<Callout>,
    /// `callout_states[i]` is the `CalloutState` *entering* line `i`.
    /// Parallel to `callout_for_line`.
    callout_states: Vec<CalloutState>,
    /// `callout_counts[i]` is `callouts.len()` *entering* line `i` (i.e.
    /// how many callouts had already been created by the time line `i` was
    /// about to be processed) — `patch_callouts` needs this on top of
    /// `callout_states` alone, since a resumed walk's freshly assigned
    /// callout indices only agree with the old cached ones if it's created
    /// the *same number* of callouts by the same point, not just reached
    /// an equal `(block_open, current)` state — see that function's own
    /// doc comment.
    callout_counts: Vec<usize>,
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
            table_for_line: Vec::new(),
            tables: Vec::new(),
            callout_for_line: Vec::new(),
            callouts: Vec::new(),
            callout_states: Vec::new(),
            callout_counts: Vec::new(),
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

    pub(super) fn table_for_line(&self) -> &[Option<usize>] {
        &self.table_for_line
    }

    pub(super) fn tables(&self) -> &[Table] {
        &self.tables
    }

    pub(super) fn callout_for_line(&self) -> &[Option<usize>] {
        &self.callout_for_line
    }

    pub(super) fn callouts(&self) -> &[Callout] {
        &self.callouts
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
        self.recompute_block_preview(text);
        self.recompute_callouts(text);
        self.recompute_block_context(text);
        self.recompute_tables(text);
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
    pub(super) fn record_edit(&mut self, text: &str, edit: TextEdit) -> Option<Range<usize>> {
        let Some(span) = self.edit_span(text, edit) else {
            self.recompute_all(text);
            return None;
        };

        // Every line the edit's old byte range actually spanned has
        // different raw text now, regardless of what any classifier below
        // reports — this is the floor of what's touched.
        let minimal = span.start_line..(span.start_line + span.lines_removed + 1);

        let r1 = self.patch_block_preview(text, &span);
        let r2 = self.patch_callouts(text, &span);
        // `patch_tables` relies on `line_index` already reflecting the new
        // text's layout, which `patch_block_preview` above has already
        // guaranteed.
        let r3 = self.patch_block_context(text, &span);
        let r4 = self.patch_tables(text, &span);

        let touched = [minimal, r1, r2, r3, r4].into_iter().reduce(union_ranges).unwrap();
        Some(touched.start..touched.end.min(self.line_roles.len()))
    }

    fn recompute_block_preview(&mut self, text: &str) {
        let results = block_preview::classify_lines_from(text, None);
        let n = results.len();
        self.line_roles = results.iter().map(|(role, _)| *role).collect();
        self.fence_states = Vec::with_capacity(n);
        self.fence_states.push(None);
        self.fence_states.extend(results.iter().take(n.saturating_sub(1)).map(|(_, state)| *state));
        self.line_index = build_line_index(text);
    }

    fn recompute_callouts(&mut self, text: &str) {
        let mut callouts = Vec::new();
        let results = callout::classify_callouts_from(text, CalloutState::default(), 0, &mut callouts);
        let n = results.len();
        self.callout_for_line = results.iter().map(|(callout, _, _)| *callout).collect();
        self.callout_states = Vec::with_capacity(n);
        self.callout_states.push(CalloutState::default());
        self.callout_states.extend(results.iter().take(n.saturating_sub(1)).map(|(_, state, _)| *state));
        self.callout_counts = Vec::with_capacity(n);
        self.callout_counts.push(0);
        self.callout_counts.extend(results.iter().take(n.saturating_sub(1)).map(|(_, _, count)| *count));
        self.callouts = callouts;
    }

    fn recompute_block_context(&mut self, text: &str) {
        let results = live_preview::classify_block_context_from(text, OpenBlock::None);
        let n = results.len();
        self.block_contexts = results.iter().map(|(ctx, _)| *ctx).collect();
        self.open_block_states = Vec::with_capacity(n);
        self.open_block_states.push(OpenBlock::None);
        self.open_block_states.extend(results.into_iter().take(n.saturating_sub(1)).map(|(_, state)| state));
    }

    fn recompute_tables(&mut self, text: &str) {
        let (table_for_line, tables) = table_preview::classify_tables(text);
        self.table_for_line = table_for_line;
        self.tables = tables;
    }

    /// The byte offset right after line `line`'s own content, excluding
    /// its trailing `\n` (or the document's total length, for the actual
    /// last line, which has none) — the complement of [`Self::line_start_
    /// byte`].
    pub(super) fn line_end_byte(&self, line: usize) -> usize {
        let next_start = self.line_index.prefix_sum(line + 1) as usize;
        if line + 1 < self.line_roles.len() { next_start - 1 } else { next_start }
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
    fn patch_tables(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.table_for_line.len();
        let start_line = span.start_line;
        let last_touched_line = (start_line + span.lines_removed).min(old_line_count.saturating_sub(1));

        let touched_old_table = (start_line..=last_touched_line).any(|i| self.table_for_line[i].is_some());
        let touched_new_pipe_row = (start_line..=last_touched_line).any(|i| is_pipe_row(self.line_text_at(text, i)));
        if !touched_old_table && !touched_new_pipe_row {
            // No table's own *content* could have changed, but an edit
            // anywhere before a table still shifts every subsequent byte
            // offset — `line_index` already reflects that (patched before
            // `patch_tables` runs), so every table's `byte_range` still
            // needs refreshing from it, even though nothing about which
            // lines belong to which table changes. Caught by this
            // module's own property test (the "far from any table" case),
            // not reasoned out in advance — an earlier version of this
            // function treated "nothing table-related touched" as a
            // literal no-op and left every table's `byte_range` stale.
            let new_ranges: Vec<_> = self
                .tables
                .iter()
                .map(|t| self.line_index.prefix_sum(t.start_line) as usize..self.line_end_byte(t.end_line - 1))
                .collect();
            for (t, range) in self.tables.iter_mut().zip(new_ranges) {
                t.byte_range = range;
            }
            // Table *membership* didn't change for any line here — only
            // pre-existing tables' byte ranges were refreshed — so nothing
            // beyond the minimal raw-text-changed span needs its row count
            // re-derived on this classifier's account.
            return start_line..(last_touched_line + 1);
        }

        let mut window_start = start_line;
        let mut window_end = last_touched_line + 1;
        for i in start_line..=last_touched_line {
            if let Some(idx) = self.table_for_line[i] {
                window_start = window_start.min(self.tables[idx].start_line);
                window_end = window_end.max(self.tables[idx].end_line);
            }
        }
        while window_start > 0 && is_pipe_row(self.line_text_at(text, window_start - 1)) {
            window_start -= 1;
        }
        while window_end < old_line_count && is_pipe_row(self.line_text_at(text, window_end)) {
            window_end += 1;
        }

        let window_start_byte = self.line_index.prefix_sum(window_start) as usize;
        let window_end_byte = self.line_end_byte(window_end - 1);
        let window_text = &text[window_start_byte..window_end_byte];
        let (local_for_line, local_tables) = table_preview::classify_tables(window_text);

        let prefix_table_count = self.tables.partition_point(|t| t.end_line <= window_start);
        let suffix_start_idx = self.tables.partition_point(|t| t.start_line < window_end);

        let mut new_tables: Vec<Table> = self.tables[..prefix_table_count].to_vec();
        for t in &local_tables {
            new_tables.push(Table {
                start_line: t.start_line + window_start,
                end_line: t.end_line + window_start,
                byte_range: (t.byte_range.start + window_start_byte)..(t.byte_range.end + window_start_byte),
                columns: t.columns.clone(),
            });
        }
        let first_kept_suffix_new_idx = new_tables.len();
        for t in &self.tables[suffix_start_idx..] {
            new_tables.push(Table {
                start_line: t.start_line,
                end_line: t.end_line,
                byte_range: self.line_index.prefix_sum(t.start_line) as usize..self.line_end_byte(t.end_line - 1),
                columns: t.columns.clone(),
            });
        }

        for (i, entry) in local_for_line.iter().enumerate() {
            self.table_for_line[window_start + i] = entry.map(|idx| idx + prefix_table_count);
        }
        for (offset, t) in new_tables[first_kept_suffix_new_idx..].iter().enumerate() {
            let new_idx = first_kept_suffix_new_idx + offset;
            for entry in &mut self.table_for_line[t.start_line..t.end_line] {
                *entry = Some(new_idx);
            }
        }

        self.tables = new_tables;
        window_start..window_end
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
    /// `None` whenever the edit changed the document's total line count:
    /// `line_index` (and every per-classifier entering-state array) is a
    /// fixed-size structure indexed by old line number, with no insert/
    /// remove operation — once line indices shift there's no single old
    /// "corresponding line" left for a patch function to compare a resumed
    /// line's state against. An accepted, explicitly-scoped gap (see
    /// CLAUDE.md's Backlog), not an oversight — Enter, a multi-line paste,
    /// or deleting across a newline all still work correctly, just via the
    /// slower whole-classifier recompute for now.
    fn edit_span(&self, text: &str, edit: TextEdit) -> Option<EditSpan> {
        if self.line_roles.is_empty() {
            return None;
        }
        let start_line = self.line_at(edit.start_byte);
        let end_line = self.line_at(edit.old_end_byte);
        let lines_removed = end_line.saturating_sub(start_line);
        let lines_added = text[edit.start_byte..edit.new_end_byte].matches('\n').count();
        (lines_added == lines_removed).then_some(EditSpan { start_line, lines_removed })
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
        let last_touched_line = start_line + span.lines_removed;

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = self.fence_states[start_line];
        for (i, (line, line_len, has_next_line)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (role, next_state) = block_preview::step(state, line);
            self.line_roles[i] = role;

            let old_len = self.line_index.prefix_sum(i + 1) - self.line_index.prefix_sum(i);
            let new_len = line_len as i64 + if has_next_line { 1 } else { 0 };
            self.line_index.add_at(i, new_len - old_len);

            let has_next = i + 1 < old_line_count;
            let reconverged = has_next && i >= last_touched_line && self.fence_states[i + 1] == next_state;
            if has_next {
                self.fence_states[i + 1] = next_state;
            }
            if reconverged || !has_next {
                return start_line..(i + 1);
            }
            state = next_state;
        }
        start_line..old_line_count
    }

    /// The real incremental path for `callout`: the same resumable-walk
    /// shape as `patch_block_preview`, with two extra wrinkles:
    ///
    /// 1. A callout's index into `self.callouts` is only meaningful
    ///    relative to how many callouts exist *before* it, so reconvergence
    ///    needs the resumed walk's running `callouts.len()` to match what
    ///    the old cached `callout_counts` recorded at the same line, not
    ///    just a matching `CalloutState`. Two states can coincidentally be
    ///    equal (e.g. both `(block_open: false, current: None)`) while a
    ///    different number of callouts were created to reach them, which
    ///    would otherwise splice a stale tail with every index off by
    ///    however many callouts diverged.
    /// 2. If the callout that's still open *at* the reconvergence point
    ///    continues (unchanged) into the lines being skipped, its `end_line`
    ///    as this walk left it only reflects the lines actually walked, not
    ///    its true full extent — `old_snapshot` (captured before any
    ///    mutation) is what restores that one field to its real value,
    ///    since the untouched tail beyond the reconvergence point is what
    ///    would otherwise have kept extending it. Caught by this module's
    ///    own randomized property test, not reasoned out in advance.
    fn patch_callouts(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.callout_for_line.len();
        let start_line = span.start_line;
        let last_touched_line = start_line + span.lines_removed;

        let prefix_callout_count = self.callout_counts[start_line];
        let old_snapshot: Vec<Callout> = self.callouts.clone();
        self.callouts.truncate(prefix_callout_count);

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = self.callout_states[start_line];
        for (i, (line, ..)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (callout_idx, next_state) = callout::step(state, line, i, &mut self.callouts);
            self.callout_for_line[i] = callout_idx;

            let has_next = i + 1 < old_line_count;
            let reconverged = has_next
                && i >= last_touched_line
                && self.callout_states[i + 1] == next_state
                && self.callouts.len() == self.callout_counts[i + 1];
            if has_next {
                self.callout_states[i + 1] = next_state;
                self.callout_counts[i + 1] = self.callouts.len();
            }
            if reconverged {
                if let Some(idx) = next_state.current {
                    self.callouts[idx].end_line = old_snapshot[idx].end_line;
                }
                self.callouts.extend_from_slice(&old_snapshot[self.callouts.len()..]);
                return start_line..(i + 1);
            }
            if !has_next {
                // Walked to the document's actual last line with no
                // reconvergence — `self.callouts` is already the complete,
                // freshly computed result, and `old_snapshot` (everything
                // that used to follow) is correctly discarded.
                return start_line..(i + 1);
            }
            state = next_state;
        }
        start_line..old_line_count
    }

    /// The real incremental path for `live_preview`'s block context: the
    /// same resumable-walk shape as `patch_block_preview` — `OpenBlock`
    /// never references an external index the way a callout's state does,
    /// so (unlike `patch_callouts`) a plain state-equality check at
    /// reconvergence is enough on its own.
    fn patch_block_context(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.block_contexts.len();
        let start_line = span.start_line;
        let last_touched_line = start_line + span.lines_removed;

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = self.open_block_states[start_line].clone();
        for (i, (line, ..)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (ctx, next_state) = live_preview::block_context_step(state, line);
            self.block_contexts[i] = ctx;

            let has_next = i + 1 < old_line_count;
            let reconverged = has_next && i >= last_touched_line && self.open_block_states[i + 1] == next_state;
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

/// The smallest range containing both `a` and `b` — used by
/// [`DocumentCache::record_edit`] to union each classifier's own touched
/// range into one overall "these lines might need their row heights
/// recomputed" range.
fn union_ranges(a: Range<usize>, b: Range<usize>) -> Range<usize> {
    a.start.min(b.start)..a.end.max(b.end)
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

/// Per-line byte strides for `text` — `line_text.len() + 1` for every line
/// but the last (which has no trailing `\n`) — wrapped in a [`Fenwick`]
/// tree so `prefix_sum(i)` gives line `i`'s own byte offset and `find`
/// answers "which line contains byte offset X" in O(log n).
fn build_line_index(text: &str) -> Fenwick {
    let mut strides = Vec::new();
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let has_next = lines.peek().is_some();
        strides.push(line.len() as i64 + if has_next { 1 } else { 0 });
    }
    Fenwick::new(&strides)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies one edit to `text` (replacing `start..old_end` with `insert`)
    /// and returns the resulting text alongside the `TextEdit` describing
    /// it — the same shape every real mutation call site in `editor.rs`/
    /// `input_handler.rs` constructs, just built directly here instead of
    /// going through a real `EditorBuffer`.
    fn apply_edit(text: &str, start: usize, old_end: usize, insert: &str) -> (String, TextEdit) {
        let mut new_text = String::with_capacity(text.len() - (old_end - start) + insert.len());
        new_text.push_str(&text[..start]);
        new_text.push_str(insert);
        new_text.push_str(&text[old_end..]);
        let edit = TextEdit { start_byte: start, old_end_byte: old_end, new_end_byte: start + insert.len() };
        (new_text, edit)
    }

    /// The property `patch_block_preview` actually relies on: after any
    /// sequence of edits (including ones that change the document's line
    /// count, forcing the fallback path), the incrementally-maintained
    /// `line_roles` must equal calling `classify_lines` fresh on the
    /// resulting text — the "oracle" `block_preview.rs`'s own module doc
    /// now describes it as.
    #[test]
    fn incremental_block_preview_matches_the_oracle_after_a_hand_picked_sequence_of_edits() {
        let mut text = String::from("intro\n```\ncode line one\ncode line two\n```\nplain again\n> quote\n- item");
        let mut cache = DocumentCache::new(&text);
        assert_eq!(cache.line_roles(), block_preview::classify_lines(&text).as_slice());

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
            assert_eq!(cache.line_roles(), expected.as_slice(), "after editing {:?}", (start, old_end, insert));
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
        assert_eq!(cache.tables().len(), 2, "sanity: two separate tables to start");

        // Delete the blank line between the two tables — `classify_tables`
        // would now see one unbroken pipe-row run, but since "| 1 | 2 |"
        // isn't a valid delimiter row for a 2-column header, this merges
        // into a *single* table (the first one's body absorbing
        // everything through the second table's own rows) rather than two.
        let blank_line_start = text.find("\n\n| c").unwrap() + 1;
        let (new_text, edit) = apply_edit(&text, blank_line_start, blank_line_start + 1, "");
        cache.record_edit(&new_text, edit);
        let (expected_for_line, expected_tables) = table_preview::classify_tables(&new_text);
        assert_eq!(cache.table_for_line(), expected_for_line.as_slice());
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
        assert_eq!(cache.table_for_line(), expected_for_line.as_slice());
        assert_eq!(cache.tables(), expected_tables.as_slice());
        text = new_text;

        // An edit far from anything table-related (the leading "before")
        // must be a complete no-op for this classifier.
        let (new_text, edit) = apply_edit(&text, 0, 0, "BEFORE ");
        cache.record_edit(&new_text, edit);
        let (expected_for_line, expected_tables) = table_preview::classify_tables(&new_text);
        assert_eq!(cache.table_for_line(), expected_for_line.as_slice());
        assert_eq!(cache.tables(), expected_tables.as_slice());
    }

    /// Same property, exercised over many pseudo-random edits — deliberate
    /// coverage of positions/fragments a hand-picked sequence wouldn't
    /// think to try, same "fuzz the invariant against the oracle" approach
    /// `fenwick.rs`'s own random test uses. Covers both `line_roles` and
    /// `callout_for_line`/`callouts` together, since every edit exercises
    /// both patch functions at once via the same `record_edit` call.
    #[test]
    fn incremental_classification_matches_the_oracle_across_many_randomized_edits() {
        let mut seed: u64 = 0xC0FF_EE15_EA5E_D00Du64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };

        let mut text = String::from(
            "intro\n```\ncode\n```\nplain\n> q\n- item\n  - nested\n    - deeper\nmore plain text here\n~~~\ntilde\n~~~\nend\n\
             > [!info]\n> body\n\n> [!warning] custom\n> more body\nafter\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\nafter table",
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
        ];

        for step in 0..400 {
            let start = (next() as usize) % (text.len() + 1);
            let max_delete = text.len() - start;
            let delete_len = if max_delete == 0 { 0 } else { (next() as usize) % (max_delete + 1) };
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

            let (expected_callout_for_line, expected_callouts) = callout::classify_callouts(&new_text);
            assert_eq!(
                cache.callout_for_line(),
                expected_callout_for_line.as_slice(),
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

            let (expected_table_for_line, expected_tables) = table_preview::classify_tables(&new_text);
            assert_eq!(
                cache.table_for_line(),
                expected_table_for_line.as_slice(),
                "table_for_line step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );
            assert_eq!(
                cache.tables(),
                expected_tables.as_slice(),
                "tables step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );

            text = new_text;
        }
    }
}

//! The incremental classifier patches: after an edit, each walks forward from
//! the edited line (or, for tables, rescans a bounded window) and stops where
//! its result lines up with the cached one again.

use super::*;

/// What `patch_tables` must learn from the *old* table layout before the
/// edit's line shift destroys it.
pub(super) struct TablePre {
    /// Whether any old line the edit replaced belonged to a table.
    touched_old_table: bool,
    /// The union of the old line extents of every table that did.
    old_extent: Option<(usize, usize)>,
}

impl DocumentCache {
    /// The table whose lines include `line`, found by line number (the
    /// `table_for_line` indices are stale for tables after a dropped one).
    pub(super) fn table_containing(&self, line: usize) -> Option<&Table> {
        let i = self.tables.partition_point(|t| t.end_line <= line);
        self.tables.get(i).filter(|t| t.start_line <= line)
    }

    /// Reads, from the *old* layout, which tables the edit's replaced lines
    /// belonged to. Must run before [`Self::resize_lines`].
    pub(super) fn table_pre(&self, span: &EditSpan) -> TablePre {
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
    pub(super) fn tables_touching(&self, span: &EditSpan) -> (usize, usize) {
        let old_last = (span.start_line + span.lines_removed).min(self.line_roles.len() - 1);
        let first = self
            .tables
            .partition_point(|t| t.end_line <= span.start_line);
        let end = self.tables.partition_point(|t| t.start_line <= old_last);
        (first, end.max(first))
    }

    /// The real incremental path for `table::model`: unlike the other
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
    /// table either way (`table::classify_tables`'s own row-
    /// consumption loop never starts a second one mid-run).
    ///
    /// `table::classify_tables` (the existing, already-correct
    /// whole-document function) is reused directly on just that window's
    /// substring, then spliced back in: tables entirely before/after the
    /// window are kept, with only `table_for_line` entries actually inside
    /// the window or belonging to a kept-after table whose index shifted
    /// touched — bounded by the window's own size plus the total number of
    /// tables and the line spans of ones kept, not by document length.
    pub(super) fn patch_tables(
        &mut self,
        text: &str,
        span: &EditSpan,
        pre: &TablePre,
    ) -> Range<usize> {
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
        let local_tables = table::classify_table_list(window_text);

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
    pub(super) fn patch_columns(
        &mut self,
        text: &str,
        span: &EditSpan,
        roles: &Range<usize>,
    ) -> Range<usize> {
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

    /// The real incremental path for `preview::fence`: resumes from the
    /// edited line's cached entering fence state, walks forward over the
    /// *new* text (one line at a time, via [`lines_from`] — never
    /// materializing the whole document's lines, see this module's own doc
    /// comment) re-running [`preview::fence::step`], and stops the moment a
    /// freshly computed exit state matches what was already cached for the
    /// next line — everything after that point is provably identical to
    /// before (same state in, same unchanged text, same result), so it's
    /// reused as-is rather than recomputed.
    pub(super) fn patch_block_preview(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
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
        let mut state = preview::fence::unpack_fence(self.fence_states[start_line]);
        for (i, (line, line_len, has_next_line)) in (start_line..).zip(lines_from(text, start_byte))
        {
            let (role, next_state) = preview::fence::step(state, line);
            self.line_roles[i] = role;

            let old_len = self.line_index.prefix_sum(i + 1) - self.line_index.prefix_sum(i);
            let new_len = line_len as i64 + if has_next_line { 1 } else { 0 };
            self.line_index.add_at(i, new_len - old_len);

            let has_next = i + 1 < old_line_count;
            let packed_next = preview::fence::pack_fence(next_state);
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
    pub(super) fn patch_callouts(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
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
            let (_, next_state) = preview::callout::step(state, line, i, &mut fresh);
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

    /// The real incremental path for `preview`'s block context: the
    /// same resumable-walk shape as `patch_block_preview` — `OpenBlock`
    /// never references an external index the way a callout's state does,
    /// so (unlike `patch_callouts`) a plain state-equality check at
    /// reconvergence is enough on its own.
    pub(super) fn patch_block_context(&mut self, text: &str, span: &EditSpan) -> Range<usize> {
        let old_line_count = self.block_contexts.len();
        let start_line = span.start_line;
        let last_touched_line = span.last_new_line();

        let start_byte = self.line_index.prefix_sum(start_line) as usize;
        let mut state = self.open_block_states[start_line].clone();
        for (i, (line, ..)) in (start_line..).zip(lines_from(text, start_byte)) {
            let (ctx, next_state) = preview::block_context_step(state, line);
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
pub(super) fn extent_touching(regions: &[Columns], a: usize, b: usize) -> Option<(usize, usize)> {
    // Regions are disjoint and sorted, so both ends are sorted too.
    let first = regions.partition_point(|r| r.end_line <= a);
    let end = regions.partition_point(|r| r.start_line < b).max(first);
    let touching = &regions[first..end];
    Some((touching.first()?.start_line, touching.last()?.end_line))
}

pub(super) fn columns_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// A callout the user folded or unfolded keeps that state when an edit
/// re-derives it, matched by its header line and its marker (a `-` rewritten
/// to `+` starts over) (the walk restarts every callout
/// it touches from the marker's own state). `old_last` is the last touched
/// line in the old numbering; lines after it moved by `delta`.
pub(super) fn carry_fold_state(
    fresh: &mut [Callout],
    old: &[Callout],
    old_last: usize,
    delta: isize,
) {
    let moved = |line: usize| {
        if line <= old_last {
            line
        } else {
            shift_line(line, delta)
        }
    };
    for c in fresh {
        let kept = old.iter().find(|old| {
            old.foldable
                && c.foldable
                && old.marker_folded == c.marker_folded
                && moved(old.start_line) == c.start_line
        });
        if let Some(old) = kept {
            c.folded = old.folded;
        }
    }
}

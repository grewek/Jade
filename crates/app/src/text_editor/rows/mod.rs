//! The row-count cache: how many wrapped rows each line takes, kept up to date
//! across edits, cursor moves and resizes without shaping the document.
//!
//! The rest of the editor sees [`RowCache`] and three small inputs:
//! [`Inputs`] (what a count depends on besides the document), [`Change`] (what
//! happened since the last update) and [`Invalidate`] (what to forget).

mod count;

use std::cell::RefCell;
use std::ops::Range;

use gpui_kit::*;

use crate::text_editor::document_cache::{DocumentCache, EditPatch};
use crate::text_editor::element::region;
use crate::text_editor::fenwick::Fenwick;
use crate::text_editor::layout::mono_char_width;
use count::{
    DocView, RowParams, active_table, patched_row_count, pure_row_counts, row_count_for_line,
    row_owner, shaped_row_count_for_line,
};

/// What a row count depends on besides the document's structure.
pub(super) struct Inputs<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub focused: bool,
    /// The width lines wrap at (`Pixels::MAX` before the first layout).
    pub width: Pixels,
    pub font: &'a Font,
    pub font_size: Pixels,
    pub text_color: Hsla,
}

/// What happened since the cache was last brought up to date.
pub(super) enum Change<'a> {
    /// A text edit; `patch` is what `DocumentCache::record_edit` returned.
    Edit(&'a EditPatch),
    /// These lines changed in height without an edit (a region's divider
    /// dragged): re-measure them alone.
    Lines(Range<usize>),
    /// A paint is about to place lines: rebuild if the width or font changed,
    /// else patch the pending lines and the lines whose active state changed.
    Layout,
}

/// What to forget.
pub(super) enum Invalidate {
    /// Everything: the next [`RowCache::update`] rebuilds every count (as a
    /// change to what the counts depend on besides the text, an image's size,
    /// requires).
    Width,
    /// These lines, patched at the next layout.
    Lines(Range<usize>),
}

/// Everything a per-line count reads, with the derived character width.
pub(super) struct Env<'a> {
    pub text: &'a str,
    pub doc: &'a DocumentCache,
    pub cursor: usize,
    pub focused: bool,
    pub width: Pixels,
    pub char_width: Pixels,
    pub font: &'a Font,
    pub font_size: Pixels,
    pub text_color: Hsla,
}

impl<'a> Env<'a> {
    fn new(inp: &'a Inputs, doc: &'a DocumentCache, char_width: Pixels) -> Self {
        Env {
            text: inp.text,
            doc,
            cursor: inp.cursor,
            focused: inp.focused,
            width: inp.width,
            char_width,
            font: inp.font,
            font_size: inp.font_size,
            text_color: inp.text_color,
        }
    }
}

/// The width regions are laid out across for an editor `available_width`
/// wide: none before the first layout (no real width yet).
fn column_width_for(available_width: Pixels) -> Option<f32> {
    (available_width > px(0.) && available_width < Pixels::MAX).then(|| f32::from(available_width))
}

/// A persistent, edit-patched cache of every line's *row count* (how many
/// wrapped visual rows a line occupies), so a paint never re-derives them from
/// the document. Mirrors `DocumentCache`'s own shape: a whole-document pass
/// once, a patch per edit after that.
///
/// Stores the counts in a [`Fenwick`] tree (not a flat `Vec`): a point update
/// to one line shifts every later line's cumulative offset, which a flat array
/// would need an O(suffix) walk to keep in sync. `line_height` is deliberately
/// not baked in: counts are a property of wrapping alone.
///
/// Three things invalidate a count, matching what it can depend on:
/// - **An edit**: patched at the same call site `DocumentCache` is patched
///   ([`Change::Edit`], with the touched-line range).
/// - **The active line changing with no edit** (arrow keys, a click elsewhere):
///   handled lazily by [`Change::Layout`], which compares the current active
///   line against the cached one and patches at most the old and the new.
/// - **A rendering-context change** (resize, font or size): caught by
///   [`Change::Layout`] through the width and character-width fingerprint; a
///   mismatch forces a full, O(document) rebuild.
pub(super) struct RowCache {
    rows: Fenwick,
    /// The line (if any) rendered raw as of the last build or patch.
    active_line: Option<usize>,
    /// The table cell being edited as of the last build or patch: moving the
    /// caret between cells changes row counts like moving between lines does.
    editing: Option<(usize, usize)>,
    /// The width the last rebuild ran with: part of the context fingerprint.
    available_width: Pixels,
    /// The monospace advance of the last rebuild: font and size in one
    /// comparable value, without needing `Font: PartialEq`.
    char_width: Pixels,
    /// One contiguous window of lines (the last painted) whose counts are
    /// exact, not estimates. Reset by a rebuild or a line-count change.
    verified: Range<usize>,
    /// Corrections paint still had to make after [`Self::verify`]: there
    /// should be none.
    late_corrections: usize,
    /// Lines whose counts changed without an edit (a callout folded or opened),
    /// patched at the next layout instead of rebuilding every count from
    /// estimates, which moved the content height under the view.
    pending: Option<Range<usize>>,
}

impl RowCache {
    /// Before the first paint: width and character width are `0`, a sentinel no
    /// real value matches, so the first [`Change::Layout`] rebuilds.
    pub(super) fn empty() -> Self {
        Self {
            rows: Fenwick::new(&[]),
            active_line: None,
            editing: None,
            available_width: px(0.),
            char_width: px(0.),
            verified: 0..0,
            late_corrections: 0,
            pending: None,
        }
    }

    pub(super) fn line_count(&self) -> usize {
        self.rows.len()
    }

    /// The cumulative row count of lines `0..i`; `i == line_count()` gives the
    /// document's total.
    pub(super) fn prefix_sum(&self, i: usize) -> i64 {
        self.rows.prefix_sum(i)
    }

    /// Forgets counts; see [`Invalidate`].
    pub(super) fn invalidate(&mut self, what: Invalidate) {
        match what {
            Invalidate::Width => self.available_width = px(0.),
            Invalidate::Lines(lines) => {
                self.pending = Some(match self.pending.take() {
                    Some(old) => old.start.min(lines.start)..old.end.max(lines.end),
                    None => lines,
                });
            }
        }
    }

    /// Brings the counts up to date with `change`. Returns whether every count
    /// was rebuilt (the caller may then need another frame for the new extent).
    ///
    /// A stale cache (never built, a different line count, width or font) is
    /// rebuilt in full, setting the width regions are laid out across on `doc`:
    /// it is always the width the rows were built for. The exception is
    /// [`Change::Lines`], which only invalidates: the next paint rebuilds.
    pub(super) fn update(
        &mut self,
        doc: &RefCell<DocumentCache>,
        change: Change,
        inp: &Inputs,
        window: &mut Window,
    ) -> bool {
        let char_width = mono_char_width(inp.font, inp.font_size, inp.text_color, window);
        if let Change::Edit(patch) = &change {
            self.resize_for(patch, &doc.borrow());
        }
        let stale = self.line_count() != doc.borrow().line_count()
            || self.available_width != inp.width
            || self.char_width != char_width;
        if stale {
            if matches!(change, Change::Lines(_)) {
                self.invalidate(Invalidate::Width);
                return false;
            }
            doc.borrow_mut()
                .set_column_width(column_width_for(inp.width));
            self.rebuild(&doc.borrow(), inp, char_width, window);
            return true;
        }
        let doc = doc.borrow();
        let env = Env::new(inp, &doc, char_width);
        match change {
            Change::Edit(patch) => self.patch(&env, patch.touched.clone(), window),
            Change::Lines(lines) => self.patch(&env, lines, window),
            Change::Layout => {
                if let Some(lines) = self.pending.take() {
                    self.patch(&env, lines, window);
                    // Patched counts are estimates for plain lines: paint
                    // verifies the visible ones again.
                    self.verified = 0..0;
                }
                self.sync_active_line(&env, window);
            }
        }
        false
    }

    /// Makes the counts of `lines` exact. A cold rebuild seeds ordinary lines
    /// with an over-estimate that painting only corrects line by line, *after*
    /// anything positioned from the estimates (a callout's background) has
    /// been drawn. Once the real width is known a one-row estimate is already
    /// exact (it never undercounts and a line has at least one row), so only
    /// multi-row lines are shaped, once per window. Before that (the first
    /// frame wraps at `Pixels::MAX`, so every line estimates one row) every
    /// line in the window is shaped. `inp.width` is the width painting wraps at.
    pub(super) fn verify(
        &mut self,
        doc: &DocumentCache,
        lines: Range<usize>,
        inp: &Inputs,
        window: &mut Window,
    ) {
        let verified = self.verified.clone();
        if lines.start >= verified.start && lines.end <= verified.end {
            return;
        }
        let width_unknown = self.available_width >= Pixels::MAX;
        let char_width = mono_char_width(inp.font, inp.font_size, inp.text_color, window);
        let env = Env::new(inp, doc, char_width);
        let mut corrections = Vec::new();
        for i in lines.clone().filter(|i| !verified.contains(i)) {
            // A side-by-side region's rows come from shaping its columns, not
            // its first line.
            if let Some((region, split)) = doc.side_by_side(i) {
                if i == region.start_line {
                    let shaped = region::shape_region(
                        inp.text,
                        doc,
                        (&region, &split),
                        (inp.cursor, inp.focused),
                        (inp.font, inp.font_size, window.line_height()),
                        (inp.text_color.into(), None),
                        window,
                    );
                    corrections.push((i, shaped.rows));
                }
                continue;
            }
            if width_unknown || self.row_count(i) > 1 {
                corrections.push((i, shaped_row_count_for_line(&env, i, window)));
            }
        }
        for (i, count) in corrections {
            self.correct(i, count);
        }
        let touching = lines.start <= verified.end && verified.start <= lines.end;
        self.verified = if touching && !verified.is_empty() {
            lines.start.min(verified.start)..lines.end.max(verified.end)
        } else {
            lines
        };
    }

    /// Paint's per-line fix: sets line `i` to `real_rows` (exact, from shaping
    /// it), counting a late correction when that changes anything.
    pub(super) fn correct_late(&mut self, i: usize, real_rows: usize) {
        if i < self.line_count() && self.row_count(i) != real_rows as i64 {
            self.late_corrections += 1;
        }
        self.correct(i, real_rows);
    }

    /// For the paint trace: the width the counts were built for (`None` while
    /// unknown), the verified window and the late corrections so far.
    pub(super) fn trace_state(&self) -> (Option<Pixels>, Range<usize>, usize) {
        let width = (self.available_width < Pixels::MAX).then_some(self.available_width);
        (width, self.verified.clone(), self.late_corrections)
    }

    /// Full O(document) rebuild: paid on the first paint of a note, a resize or
    /// a font change. Ordinary lines get a cheap, allocation-free over-estimate
    /// (`estimate_row_count`), corrected to the exact value the first time they
    /// are painted; only table rows and delimiters, where an estimate from the
    /// raw length is not safely an over-estimate, take the exact computation.
    fn rebuild(
        &mut self,
        doc: &DocumentCache,
        inp: &Inputs,
        char_width: Pixels,
        window: &mut Window,
    ) {
        let starts = doc.line_start_offsets();
        let view = DocView {
            roles: doc.line_roles(),
            contexts: doc.block_contexts(),
            tables: doc.tables(),
            callouts: doc.callouts(),
            images: doc.images(),
            columns: doc.columns(),
            raw_region: doc.raw_region(),
            column_width: doc.column_width(),
            starts: &starts,
        };
        let params = RowParams {
            active_table: active_table(doc, inp.cursor, inp.focused),
            available_width: inp.width,
            char_width,
            line_height: window.line_height(),
        };
        let (mut counts, shaped) = pure_row_counts(inp.text, &view, &params);
        // Lines that need the window (an active or irregular table row) are the
        // few the threads left out.
        let env = Env::new(inp, doc, char_width);
        for i in shaped {
            counts[i] = row_count_for_line(&env, i, window) as i64;
        }
        assert_eq!(counts.len(), doc.line_count());
        self.rows = Fenwick::from_vec(counts);
        self.editing = doc.editing_cell();
        self.active_line = inp.focused.then(|| doc.line_at(inp.cursor));
        self.available_width = inp.width;
        self.char_width = char_width;
        self.verified = 0..0;
        self.pending = None;
    }

    /// Mirrors an edit's line-count change in the counts, so [`Self::patch`]
    /// can re-derive just the touched lines. A no-op when the cache was not in
    /// sync with the document before the edit: it is then stale and rebuilt.
    fn resize_for(&mut self, patch: &EditPatch, doc: &DocumentCache) {
        let old_count = doc.line_count() as isize - patch.line_delta;
        if patch.line_delta != 0 && self.line_count() as isize == old_count {
            self.rows.resize(patch.splice_at, patch.line_delta);
            self.verified = 0..0;
        }
    }

    /// Recomputes every line in `touched`, patching the tree in place.
    fn patch(&mut self, env: &Env, touched: Range<usize>, window: &mut Window) {
        let mut last = usize::MAX;
        for i in touched {
            // A line in a side-by-side region changes its region's height.
            let i = row_owner(env.doc, i);
            if i == last {
                continue;
            }
            last = i;
            let old = self.row_count(i);
            let new = patched_row_count(env, i, window);
            self.rows.add_at(i, new as i64 - old);
        }
        self.editing = env.doc.editing_cell();
        self.active_line = env.focused.then(|| env.doc.line_at(env.cursor));
    }

    /// Patches whichever of the previously and the newly active line (and edited
    /// table cell) changed since the last build or patch: at most four lines.
    /// Free when nothing changed.
    fn sync_active_line(&mut self, env: &Env, window: &mut Window) {
        let doc = env.doc;
        let new_active = env.focused.then(|| doc.line_at(env.cursor));
        let new_editing = doc.editing_cell();
        if new_active == self.active_line && new_editing == self.editing {
            return;
        }
        let editing_lines = [self.editing, new_editing].map(|e| e.map(|(line, _)| line));
        let lines = [self.active_line, new_active]
            .into_iter()
            .chain(editing_lines);
        let mut seen = Vec::with_capacity(4);
        for line in lines.flatten() {
            let line = row_owner(doc, line);
            if seen.contains(&line) {
                continue;
            }
            seen.push(line);
            if line < self.line_count() {
                let old = self.row_count(line);
                let new = patched_row_count(env, line, window);
                self.rows.add_at(line, new as i64 - old);
            }
        }
        self.active_line = new_active;
        self.editing = new_editing;
    }

    fn row_count(&self, i: usize) -> i64 {
        self.rows.prefix_sum(i + 1) - self.rows.prefix_sum(i)
    }

    /// Sets line `i` to `real_rows`; a no-op when it already is.
    fn correct(&mut self, i: usize, real_rows: usize) {
        if i >= self.line_count() {
            return;
        }
        let delta = real_rows as i64 - self.row_count(i);
        if delta != 0 {
            self.rows.add_at(i, delta);
        }
    }
}

/// The exact count of line `i` as a cold rebuild would compute it, for tests to
/// compare the cache against.
#[cfg(test)]
pub(super) fn exact_row_count(
    inp: &Inputs,
    doc: &DocumentCache,
    i: usize,
    window: &mut Window,
) -> usize {
    let char_width = mono_char_width(inp.font, inp.font_size, inp.text_color, window);
    row_count_for_line(&Env::new(inp, doc, char_width), i, window)
}

/// Test seams over the private build and patch paths, so a test can set a
/// cache up with exact counts and then patch it line by line.
#[cfg(test)]
pub(super) fn rebuild_for_test(
    rows: &mut RowCache,
    text: &str,
    doc: &DocumentCache,
    (cursor, focused): (usize, bool),
    width: Pixels,
    (font, font_size, text_color): (&Font, Pixels, Hsla),
    window: &mut Window,
) {
    let inp = Inputs {
        text,
        cursor,
        focused,
        width,
        font,
        font_size,
        text_color,
    };
    let char_width = mono_char_width(font, font_size, text_color, window);
    rows.rebuild(doc, &inp, char_width, window);
}

#[cfg(test)]
pub(super) fn patch_for_test(
    rows: &mut RowCache,
    text: &str,
    doc: &DocumentCache,
    lines: Range<usize>,
    (cursor, focused): (usize, bool),
    (font, font_size, text_color): (&Font, Pixels, Hsla),
    window: &mut Window,
) {
    let inp = Inputs {
        text,
        cursor,
        focused,
        width: rows.available_width,
        font,
        font_size,
        text_color,
    };
    let char_width = mono_char_width(font, font_size, text_color, window);
    rows.patch(&Env::new(&inp, doc, char_width), lines, window);
}

#[cfg(test)]
pub(super) fn sync_for_test(
    rows: &mut RowCache,
    text: &str,
    doc: &DocumentCache,
    (cursor, focused): (usize, bool),
    (font, font_size, text_color): (&Font, Pixels, Hsla),
    window: &mut Window,
) {
    let inp = Inputs {
        text,
        cursor,
        focused,
        width: rows.available_width,
        font,
        font_size,
        text_color,
    };
    let char_width = mono_char_width(font, font_size, text_color, window);
    rows.sync_active_line(&Env::new(&inp, doc, char_width), window);
}

#[cfg(test)]
pub(super) fn count_at(rows: &RowCache, i: usize) -> i64 {
    rows.row_count(i)
}

#[cfg(test)]
pub(super) fn correct_for_test(rows: &mut RowCache, i: usize, real_rows: usize) {
    rows.correct(i, real_rows);
}

#[cfg(test)]
mod tests {
    use super::{Invalidate, RowCache};

    /// Lines marked changed are patched together (a fold then a second one
    /// before a layout covers both).
    #[test]
    fn invalidated_lines_merge_into_one_range() {
        let mut rows = RowCache::empty();
        rows.invalidate(Invalidate::Lines(10..14));
        rows.invalidate(Invalidate::Lines(3..5));
        assert_eq!(rows.pending, Some(3..14));
    }
}

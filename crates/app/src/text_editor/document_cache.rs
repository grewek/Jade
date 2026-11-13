//! A persistent, edit-patched cache of the four whole-document Live Preview
//! classification passes (`block_preview::classify_lines`,
//! `live_preview::classify_block_context`, `table_preview::classify_tables`,
//! `callout::classify_callouts`) — see CLAUDE.md's Backlog entry on
//! incremental classification for why this exists: re-running all four from
//! scratch on every paint cost ~420ms combined on the 320K-line stress
//! fixture, re-paid on every keystroke, nowhere near the 16.66ms/60fps bar.
//!
//! **Real incremental paths landed for `block_preview` and `callout`** —
//! see `patch_block_preview`/`patch_callouts`. `live_preview`'s block
//! context and `table_preview`'s tables are still fully recomputed on every
//! edit (`recompute_rest`) — later increments replace each one's piece the
//! same way, one at a time, without needing to change this module's public
//! shape or any caller again.
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

use super::block_preview::{self, FenceState, LineRole};
use super::callout::{self, Callout, CalloutState};
use super::fenwick::Fenwick;
use super::live_preview::{self, LineContext};
use super::table_preview::{self, Table};

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

    /// Full whole-document recompute of every classifier — the cold-open
    /// path, and the fallback whenever a patch function can't cheaply
    /// resolve a particular edit.
    pub(super) fn recompute_all(&mut self, text: &str) {
        self.recompute_block_preview(text);
        self.recompute_callouts(text);
        self.recompute_rest(text);
    }

    /// Patches the cache for a precisely-ranged edit: the real incremental
    /// path for `block_preview`/`callout` (falling back to a full recompute
    /// of just that classifier when it can't cheaply apply), full recompute
    /// for everything else (not incremental yet — see this module's own
    /// doc comment).
    pub(super) fn record_edit(&mut self, text: &str, edit: TextEdit) {
        let span = self.edit_span(text, edit);

        let patched_block_preview = span.as_ref().is_some_and(|span| self.patch_block_preview(text, span));
        if !patched_block_preview {
            self.recompute_block_preview(text);
        }

        let patched_callouts = span.as_ref().is_some_and(|span| self.patch_callouts(text, span));
        if !patched_callouts {
            self.recompute_callouts(text);
        }

        self.recompute_rest(text);
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

    fn recompute_rest(&mut self, text: &str) {
        self.block_contexts = live_preview::classify_block_context(text);
        let (table_for_line, tables) = table_preview::classify_tables(text);
        self.table_for_line = table_for_line;
        self.tables = tables;
    }

    /// The line index whose byte-stride range contains `byte_offset`,
    /// clamped to the last line if `byte_offset` is at or past the
    /// document's current total byte length (e.g. appending at the very
    /// end) — `line_index.find` itself only answers for offsets strictly
    /// inside the tree's total.
    fn line_at(&self, byte_offset: usize) -> usize {
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
    fn patch_block_preview(&mut self, text: &str, span: &EditSpan) -> bool {
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
                return true;
            }
            state = next_state;
        }
        true
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
    fn patch_callouts(&mut self, text: &str, span: &EditSpan) -> bool {
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
                return true;
            }
            if !has_next {
                // Walked to the document's actual last line with no
                // reconvergence — `self.callouts` is already the complete,
                // freshly computed result, and `old_snapshot` (everything
                // that used to follow) is correctly discarded.
                return true;
            }
            state = next_state;
        }
        true
    }
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
            "intro\n```\ncode\n```\nplain\n> q\n- item\nmore plain text here\n~~~\ntilde\n~~~\nend\n\
             > [!info]\n> body\n\n> [!warning] custom\n> more body\nafter",
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

            text = new_text;
        }
    }
}

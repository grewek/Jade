//! A persistent, edit-patched cache of the four whole-document Live Preview
//! classification passes (`block_preview::classify_lines`,
//! `live_preview::classify_block_context`, `table_preview::classify_tables`,
//! `callout::classify_callouts`) — see CLAUDE.md's Backlog entry on
//! incremental classification for why this exists: re-running all four from
//! scratch on every paint cost ~420ms combined on the 320K-line stress
//! fixture, re-paid on every keystroke, nowhere near the 16.66ms/60fps bar.
//!
//! **First real incremental path landed for `block_preview` only** — see
//! `patch_block_preview`. The other three classifiers are still fully
//! recomputed on every edit (`recompute_rest`) — later increments replace
//! each one's piece the same way, one at a time, without needing to change
//! this module's public shape or any caller again.
//!
//! `line_index` (a [`Fenwick`] tree over per-line byte strides) is the
//! shared piece every incremental classifier needs and none of the four
//! `classify_*` passes themselves care about: mapping an edit's byte offset
//! to a line index in O(log n), without counting newlines from the start of
//! the document — see `fenwick.rs`'s own doc comment for why that matters.

use super::block_preview::{self, FenceState, LineRole};
use super::callout::{self, Callout};
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
    /// last, which has no trailing `\n`) — lets `patch_block_preview` map
    /// an edit's byte offset to a line index in O(log n) instead of
    /// counting newlines from the start of the document. See `fenwick.rs`.
    line_index: Fenwick,
    block_contexts: Vec<LineContext>,
    table_for_line: Vec<Option<usize>>,
    tables: Vec<Table>,
    callout_for_line: Vec<Option<usize>>,
    callouts: Vec<Callout>,
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

    /// Full whole-document recompute of every classifier, `block_preview`
    /// included — the cold-open path, and the fallback whenever
    /// `patch_block_preview` can't cheaply resolve a particular edit.
    pub(super) fn recompute_all(&mut self, text: &str) {
        self.recompute_block_preview(text);
        self.recompute_rest(text);
    }

    /// Patches the cache for a precisely-ranged edit: the real incremental
    /// path for `block_preview` (falling back to a full recompute of just
    /// that classifier when it can't cheaply apply — see
    /// `patch_block_preview`), full recompute for everything else (not
    /// incremental yet — see this module's own doc comment).
    pub(super) fn record_edit(&mut self, text: &str, edit: TextEdit) {
        if !self.patch_block_preview(text, edit) {
            self.recompute_block_preview(text);
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

    fn recompute_rest(&mut self, text: &str) {
        self.block_contexts = live_preview::classify_block_context(text);
        let (table_for_line, tables) = table_preview::classify_tables(text);
        self.table_for_line = table_for_line;
        self.tables = tables;
        let (callout_for_line, callouts) = callout::classify_callouts(text);
        self.callout_for_line = callout_for_line;
        self.callouts = callouts;
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

    /// The real incremental path: resumes from the edited line's cached
    /// entering fence state, walks forward over the *new* text re-running
    /// [`block_preview::step`] one line at a time, and stops the moment a
    /// freshly computed exit state matches what was already cached for the
    /// next line — everything after that point is provably identical to
    /// before (same state in, same unchanged text, same result), so it's
    /// reused as-is rather than recomputed. Returns `false` (meaning the
    /// caller should fall back to `recompute_block_preview`) whenever the
    /// edit changed the document's total line count: `line_index` is a
    /// fixed-size [`Fenwick`] tree with no insert/remove operation, and
    /// once line indices shift there's no single old "corresponding line"
    /// left to compare a resumed line's state against. This is an accepted,
    /// explicitly-scoped gap (see CLAUDE.md's Backlog), not an oversight —
    /// Enter, a multi-line paste, or deleting across a newline all still
    /// work correctly, just via the slower fallback for now.
    fn patch_block_preview(&mut self, text: &str, edit: TextEdit) -> bool {
        if self.line_roles.is_empty() {
            return false;
        }
        let old_line_count = self.line_roles.len();
        let start_line = self.line_at(edit.start_byte);
        let end_line = self.line_at(edit.old_end_byte);
        let lines_removed = end_line.saturating_sub(start_line);
        let lines_added = text[edit.start_byte..edit.new_end_byte].matches('\n').count();
        if lines_added != lines_removed {
            return false;
        }

        let new_lines: Vec<&str> = text.split('\n').collect();
        if new_lines.len() != old_line_count {
            // Shouldn't happen given the check above, but this is a
            // correctness-critical path — fall back rather than risk
            // silently corrupting the cache on a derived invariant alone.
            return false;
        }

        // The last *old* line index the edit actually replaced — every new
        // line up to and including this one is guaranteed genuinely new
        // content (part of the inserted text, however the state machine
        // happens to classify it), even if a state coincidentally matches
        // the old cached one somewhere in between. Only once the walk
        // passes this point is a matching state actually trustworthy
        // evidence that "the rest is identical to before," because only
        // then is the line's own *text* (not just the machine's state)
        // guaranteed byte-identical to what produced that cached state
        // last time. Skipping this check and reconverging early on a
        // coincidental state match is exactly what caused this path to
        // keep *stale, wrong* cached roles for the lines actually replaced
        // by a multi-line, same-line-count-delta edit — caught by the
        // randomized property test below, not reasoned out in advance.
        let last_touched_line = start_line + lines_removed;

        let mut state = self.fence_states[start_line];
        let mut i = start_line;
        loop {
            let (role, next_state) = block_preview::step(state, new_lines[i]);
            self.line_roles[i] = role;
            let has_next = i + 1 < old_line_count;
            let reconverged = has_next && i >= last_touched_line && self.fence_states[i + 1] == next_state;
            if has_next {
                self.fence_states[i + 1] = next_state;
            }
            if reconverged || !has_next {
                break;
            }
            state = next_state;
            i += 1;
        }

        // Only lines actually revisited above could have a changed byte
        // length (same total line count, so every other line's own text —
        // and therefore its stride — is untouched).
        for j in start_line..=i {
            let new_len = new_lines[j].len() as i64 + if j + 1 < new_lines.len() { 1 } else { 0 };
            let old_len = self.line_index.prefix_sum(j + 1) - self.line_index.prefix_sum(j);
            self.line_index.add_at(j, new_len - old_len);
        }
        true
    }
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
    /// `fenwick.rs`'s own random test uses.
    #[test]
    fn incremental_block_preview_matches_the_oracle_across_many_randomized_edits() {
        let mut seed: u64 = 0xC0FF_EE15_EA5E_D00Du64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed
        };

        let mut text =
            String::from("intro\n```\ncode\n```\nplain\n> q\n- item\nmore plain text here\n~~~\ntilde\n~~~\nend");
        let mut cache = DocumentCache::new(&text);

        // ASCII-only, so every random byte offset below is guaranteed a
        // valid char boundary too.
        let fragments = ["", "x", "\n", "```\n", "~~~\n", "a\nb\nc", "```rust\ncode\n```", " "];

        for step in 0..300 {
            let start = (next() as usize) % (text.len() + 1);
            let max_delete = text.len() - start;
            let delete_len = if max_delete == 0 { 0 } else { (next() as usize) % (max_delete + 1) };
            let fragment = fragments[(next() as usize) % fragments.len()];
            let old_end = start + delete_len;
            let (new_text, edit) = apply_edit(&text, start, old_end, fragment);
            cache.record_edit(&new_text, edit);
            let expected = block_preview::classify_lines(&new_text);
            assert_eq!(
                cache.line_roles(),
                expected.as_slice(),
                "step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
            );
            text = new_text;
        }
    }
}

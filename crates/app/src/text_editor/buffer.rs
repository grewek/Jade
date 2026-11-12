//! A plain-text buffer with cursor/selection, independent of GPUI.
//!
//! This is the Phase 1 foundation of Jade's own editor (see `CLAUDE.md`'s
//! "Custom editor" section): a `String`-backed buffer the GPUI-facing element
//! (rendering, mouse, keyboard, IME) will drive. Kept separate and GPUI-free
//! so the editing logic itself is exhaustively unit-testable, the same
//! pure-logic-first approach every Live Preview module (`live_preview.rs`,
//! `block_preview.rs`, `table_preview.rs`) uses too.
//!
//! Cursor movement is char-boundary based, not full grapheme-cluster based
//! (a combining-mark or ZWJ emoji sequence moves one `char` at a time, not one
//! visual cluster) — a deliberate Phase 1 simplification; `unicode-segmentation`
//! would fix this but needs its own dependency approval, deferred until it's
//! actually needed.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorBuffer {
    text: String,
    /// Byte offset, always a char boundary.
    cursor: usize,
    /// The selection's other end, if a selection is active. The selection is
    /// `anchor..cursor` or `cursor..anchor`, whichever is ordered — `anchor`
    /// itself doesn't track which side is "fixed" beyond that.
    anchor: Option<usize>,
    /// The column (in chars from its line start) a run of consecutive
    /// `move_up`/`move_down` calls is aiming for, so moving up through a line
    /// too short to reach it and then back down lands on the original column
    /// again rather than wherever the short line clamped it to. Set fresh
    /// from the actual cursor on the first vertical move of a run; cleared by
    /// every other operation (even just moving left/right) since those make
    /// a new column the deliberate one.
    preferred_column: Option<usize>,
    /// One snapshot per text-modifying operation (not per cursor movement),
    /// each taken *before* that operation applied — "basic" linear undo, not
    /// a real editor's coalescing of a whole burst of typing into one step;
    /// a deliberate Phase 1 scope cut.
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
}

impl EditorBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            cursor: 0,
            anchor: None,
            preferred_column: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The active selection, normalized (`start <= end`), or `None`.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some(if anchor < self.cursor { anchor..self.cursor } else { self.cursor..anchor })
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Moves the cursor to `offset` (clamped to a char boundary), starting or
    /// extending a selection if `extend`, else collapsing any selection.
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        self.preferred_column = None;
        self.set_cursor_internal(offset, extend);
    }

    /// `set_cursor` without touching `preferred_column` — for `move_up`/
    /// `move_down`, which manage it themselves to stay sticky across a run.
    fn set_cursor_internal(&mut self, offset: usize, extend: bool) {
        let offset = clamp_to_char_boundary(&self.text, offset);
        if extend && self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        } else if !extend {
            self.anchor = None;
        }
        self.cursor = offset;
    }

    pub fn select_all(&mut self) {
        self.preferred_column = None;
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    /// Replaces the selection (if any) with `s`, else inserts it at the cursor.
    /// The cursor ends up right after the inserted text.
    pub fn insert(&mut self, s: &str) {
        self.push_undo();
        self.preferred_column = None;
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.text.replace_range(range.clone(), s);
        self.cursor = range.start + s.len();
        self.anchor = None;
    }

    /// Deletes the selection, or the one char before the cursor.
    pub fn delete_backward(&mut self) {
        if let Some(range) = self.selection() {
            self.delete_range(range);
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = prev_char_boundary(&self.text, self.cursor);
        self.delete_range(start..self.cursor);
    }

    /// Deletes the selection, or the one char after the cursor.
    pub fn delete_forward(&mut self) {
        if let Some(range) = self.selection() {
            self.delete_range(range);
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let end = next_char_boundary(&self.text, self.cursor);
        self.delete_range(self.cursor..end);
    }

    fn delete_range(&mut self, range: Range<usize>) {
        self.push_undo();
        self.preferred_column = None;
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.start, false);
            return;
        }
        let target = prev_char_boundary(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.end, false);
            return;
        }
        let target = next_char_boundary(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    /// Moves to the start of the previous word: skips any whitespace directly
    /// to the left, then the run of word/punctuation chars before that — so
    /// from the middle of a word it lands at that word's start, and from
    /// right after a word (e.g. on the trailing space) it does too, matching
    /// the usual Ctrl/Cmd+Left behavior in other editors.
    pub fn move_word_left(&mut self, extend: bool) {
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.start, false);
            return;
        }
        let target = word_boundary_backward(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    /// Moves to the end of the current word (if the cursor is inside one), or
    /// past any whitespace and to the end of the next word otherwise — the
    /// mirror of `move_word_left`, matching Ctrl/Cmd+Right.
    pub fn move_word_right(&mut self, extend: bool) {
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.end, false);
            return;
        }
        let target = word_boundary_forward(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    /// Moves up one line, keeping the cursor's current column (in chars from
    /// the line start) where the target line is long enough, else clamping to
    /// its end — and remembering that column (see `preferred_column`) so a
    /// subsequent move back down past a too-short line lands on it again.
    pub fn move_up(&mut self, extend: bool) {
        let this_line_start = line_start(&self.text, self.cursor);
        let column = self.column_for_vertical_move(this_line_start);
        if this_line_start == 0 {
            self.set_cursor_internal(0, extend);
            return;
        }
        let prev_line_start = line_start(&self.text, this_line_start - 1);
        let target = offset_at_column(&self.text, prev_line_start, column);
        self.set_cursor_internal(target, extend);
    }

    pub fn move_down(&mut self, extend: bool) {
        let this_line_start = line_start(&self.text, self.cursor);
        let column = self.column_for_vertical_move(this_line_start);
        let this_line_end = line_end(&self.text, self.cursor);
        if this_line_end >= self.text.len() {
            self.set_cursor_internal(self.text.len(), extend);
            return;
        }
        let next_line_start = this_line_end + 1;
        let target = offset_at_column(&self.text, next_line_start, column);
        self.set_cursor_internal(target, extend);
    }

    /// The column a vertical move should aim for: the running
    /// `preferred_column` if a run of them is already in progress, else the
    /// cursor's actual current column (which also starts the run).
    fn column_for_vertical_move(&mut self, this_line_start: usize) -> usize {
        let column = self
            .preferred_column
            .unwrap_or_else(|| char_column(&self.text, this_line_start, self.cursor));
        self.preferred_column = Some(column);
        column
    }

    pub fn move_to_line_start(&mut self, extend: bool) {
        let target = line_start(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    pub fn move_to_line_end(&mut self, extend: bool) {
        let target = line_end(&self.text, self.cursor);
        self.set_cursor(target, extend);
    }

    pub fn move_to_start(&mut self, extend: bool) {
        self.set_cursor(0, extend);
    }

    pub fn move_to_end(&mut self, extend: bool) {
        self.set_cursor(self.text.len(), extend);
    }

    /// Removes and returns the selected text, or `None` if nothing is selected.
    pub fn cut(&mut self) -> Option<String> {
        let range = self.selection()?;
        let cut = self.text[range.clone()].to_string();
        self.delete_range(range);
        Some(cut)
    }

    /// Returns the selected text without modifying the buffer.
    pub fn copy(&self) -> Option<String> {
        self.selection().map(|range| self.text[range].to_string())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), cursor: self.cursor, anchor: self.anchor }
    }

    /// Records the state *before* a text-modifying operation, so `undo` can
    /// return to it, and clears `redo_stack` — making a fresh edit after an
    /// undo abandon the redone-away future, same as every other editor.
    fn push_undo(&mut self) {
        self.undo_stack.push(self.snapshot());
        self.redo_stack.clear();
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.text = snapshot.text;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
        self.preferred_column = None;
    }

    /// Undoes the most recent text-modifying operation, if any. Returns
    /// whether there was one to undo.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo_stack.pop() else { return false };
        self.redo_stack.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    /// Reapplies the most recently undone operation, if any. Returns whether
    /// there was one to redo.
    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo_stack.pop() else { return false };
        self.undo_stack.push(self.snapshot());
        self.restore(snapshot);
        true
    }
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if text.is_char_boundary(offset) {
        return offset;
    }
    prev_char_boundary(text, offset)
}

fn prev_char_boundary(text: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(text: &str, pos: usize) -> usize {
    if pos >= text.len() {
        return text.len();
    }
    let mut i = pos + 1;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[derive(PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// `pos` moved left past any whitespace directly before it, then past the
/// run of same-class (word or punctuation) chars before that — i.e. to the
/// start of whatever's immediately to the left, skipping separating
/// whitespace first.
fn word_boundary_backward(text: &str, pos: usize) -> usize {
    let mut offset = pos;
    loop {
        if offset == 0 {
            return offset;
        }
        let prev = prev_char_boundary(text, offset);
        let c = text[prev..offset].chars().next().unwrap();
        if char_class(c) != CharClass::Whitespace {
            break;
        }
        offset = prev;
    }
    let prev = prev_char_boundary(text, offset);
    let class = char_class(text[prev..offset].chars().next().unwrap());
    loop {
        if offset == 0 {
            break;
        }
        let prev = prev_char_boundary(text, offset);
        let c = text[prev..offset].chars().next().unwrap();
        if char_class(c) != class {
            break;
        }
        offset = prev;
    }
    offset
}

/// The mirror of `word_boundary_backward`: `pos` moved right past any
/// whitespace directly after it, then past the run of same-class chars after
/// that.
fn word_boundary_forward(text: &str, pos: usize) -> usize {
    let mut offset = pos;
    loop {
        let Some(c) = text[offset..].chars().next() else { return offset };
        if char_class(c) != CharClass::Whitespace {
            break;
        }
        offset = next_char_boundary(text, offset);
    }
    let class = char_class(text[offset..].chars().next().unwrap());
    loop {
        let Some(c) = text[offset..].chars().next() else { break };
        if char_class(c) != class {
            break;
        }
        offset = next_char_boundary(text, offset);
    }
    offset
}

/// The byte offset of the start of the line containing `pos`.
fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |ix| ix + 1)
}

/// The byte offset of the end of the line containing `pos` (before its `\n`,
/// or the end of the text for the last line).
fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |ix| pos + ix)
}

/// The number of chars between `line_start` and `pos` (must be on the same line).
fn char_column(text: &str, line_start: usize, pos: usize) -> usize {
    text[line_start..pos].chars().count()
}

/// The byte offset `column` chars into the line starting at `line_start`,
/// clamped to that line's own end.
fn offset_at_column(text: &str, line_start: usize, column: usize) -> usize {
    let end = line_end(text, line_start);
    text[line_start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(ix, _)| line_start + ix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_at_cursor_moves_cursor_past_it() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(5, false);
        buf.insert(" world");
        assert_eq!(buf.text(), "hello world");
        assert_eq!(buf.cursor(), 11);
    }

    #[test]
    fn insert_replaces_an_active_selection() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.set_cursor(5, true);
        buf.insert("goodbye");
        assert_eq!(buf.text(), "goodbye world");
        assert_eq!(buf.cursor(), 7);
        assert!(!buf.has_selection());
    }

    #[test]
    fn delete_backward_without_selection_removes_one_char() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(5, false);
        buf.delete_backward();
        assert_eq!(buf.text(), "hell");
        assert_eq!(buf.cursor(), 4);
    }

    #[test]
    fn delete_backward_at_start_of_buffer_is_a_no_op() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(0, false);
        buf.delete_backward();
        assert_eq!(buf.text(), "hello");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn delete_forward_without_selection_removes_the_next_char() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(0, false);
        buf.delete_forward();
        assert_eq!(buf.text(), "ello");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn delete_with_active_selection_removes_the_whole_selection() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.set_cursor(6, true);
        buf.delete_backward();
        assert_eq!(buf.text(), "world");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn move_left_right_step_one_char_at_a_time() {
        let mut buf = EditorBuffer::new("abc");
        buf.set_cursor(3, false);
        buf.move_left(false);
        assert_eq!(buf.cursor(), 2);
        buf.move_right(false);
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn move_left_right_step_over_a_whole_multi_byte_char() {
        let mut buf = EditorBuffer::new("a\u{00e9}b"); // a, é (2 bytes), b
        buf.set_cursor(1, false);
        buf.move_right(false);
        assert_eq!(buf.cursor(), 3, "should land after the full 2-byte é, not inside it");
        buf.move_left(false);
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn move_left_with_an_active_selection_collapses_to_its_start() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(1, false);
        buf.set_cursor(4, true);
        buf.move_left(false);
        assert_eq!(buf.cursor(), 1);
        assert!(!buf.has_selection());
    }

    #[test]
    fn shift_arrow_extends_a_selection_from_a_fixed_anchor() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(2, false);
        buf.move_right(true);
        buf.move_right(true);
        assert_eq!(buf.selection(), Some(2..4));
        buf.move_left(true);
        assert_eq!(buf.selection(), Some(2..3));
    }

    #[test]
    fn move_to_line_start_and_end() {
        let mut buf = EditorBuffer::new("first\nsecond line\nthird");
        buf.set_cursor(10, false); // inside "second line"
        buf.move_to_line_start(false);
        assert_eq!(buf.cursor(), 6);
        buf.move_to_line_end(false);
        assert_eq!(buf.cursor(), 17);
    }

    #[test]
    fn move_up_down_preserve_column_when_the_target_line_is_long_enough() {
        let mut buf = EditorBuffer::new("abcdef\nxy\nabcdef");
        buf.set_cursor(4, false); // column 4 on line 0
        buf.move_down(false);
        assert_eq!(buf.cursor(), 9, "clamped to the end of the short middle line");
        buf.move_down(false);
        assert_eq!(buf.cursor(), 14, "back to column 4 on the long last line");
        buf.move_up(false);
        assert_eq!(buf.cursor(), 9);
        buf.move_up(false);
        assert_eq!(buf.cursor(), 4);
    }

    #[test]
    fn a_horizontal_move_resets_the_preferred_column() {
        let mut buf = EditorBuffer::new("abcdef\nxy\nabcdef");
        buf.set_cursor(4, false); // column 4 on line 0
        buf.move_down(false);
        assert_eq!(buf.cursor(), 9, "clamped to the end of the short middle line");
        buf.move_left(false); // now at column 1 on the short line, not 4
        assert_eq!(buf.cursor(), 8);
        buf.move_down(false);
        assert_eq!(buf.cursor(), 11, "column 1 on the last line, not the long-forgotten 4");
    }

    #[test]
    fn move_up_from_the_first_line_goes_to_the_very_start() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(3, false);
        buf.move_up(false);
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn move_down_from_the_last_line_goes_to_the_very_end() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(2, false);
        buf.move_down(false);
        assert_eq!(buf.cursor(), 5);
    }

    #[test]
    fn select_all_selects_the_whole_buffer() {
        let mut buf = EditorBuffer::new("hello world");
        buf.select_all();
        assert_eq!(buf.selection(), Some(0..11));
        assert_eq!(buf.cursor(), 11);
    }

    #[test]
    fn copy_leaves_the_buffer_unchanged() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.set_cursor(5, true);
        assert_eq!(buf.copy(), Some("hello".to_string()));
        assert_eq!(buf.text(), "hello world");
        assert!(buf.has_selection());
    }

    #[test]
    fn cut_removes_the_selection_and_returns_it() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.set_cursor(5, true);
        assert_eq!(buf.cut(), Some("hello".to_string()));
        assert_eq!(buf.text(), " world");
        assert_eq!(buf.cursor(), 0);
        assert!(!buf.has_selection());
    }

    #[test]
    fn cut_without_a_selection_does_nothing() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(3, false);
        assert_eq!(buf.cut(), None);
        assert_eq!(buf.text(), "hello world");
    }

    #[test]
    fn set_cursor_clamps_into_the_middle_of_a_multi_byte_char_to_its_start() {
        let mut buf = EditorBuffer::new("a\u{00e9}b");
        buf.set_cursor(2, false); // byte 2 is inside é (bytes 1..3)
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn undo_restores_text_and_cursor_from_before_an_insert() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(5, false);
        buf.insert(" world");
        assert!(buf.undo());
        assert_eq!(buf.text(), "hello");
        assert_eq!(buf.cursor(), 5);
    }

    #[test]
    fn redo_reapplies_an_undone_edit() {
        let mut buf = EditorBuffer::new("hello");
        buf.set_cursor(5, false);
        buf.insert(" world");
        buf.undo();
        assert!(buf.redo());
        assert_eq!(buf.text(), "hello world");
        assert_eq!(buf.cursor(), 11);
    }

    #[test]
    fn multiple_undos_walk_back_through_history_in_order() {
        let mut buf = EditorBuffer::new("");
        buf.insert("a");
        buf.insert("b");
        buf.insert("c");
        assert_eq!(buf.text(), "abc");
        assert!(buf.undo());
        assert_eq!(buf.text(), "ab");
        assert!(buf.undo());
        assert_eq!(buf.text(), "a");
        assert!(buf.undo());
        assert_eq!(buf.text(), "");
        assert!(!buf.undo(), "nothing left to undo");
    }

    #[test]
    fn undo_with_an_empty_history_is_a_no_op() {
        let mut buf = EditorBuffer::new("hello");
        assert!(!buf.undo());
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn a_new_edit_after_an_undo_clears_the_redo_stack() {
        let mut buf = EditorBuffer::new("");
        buf.insert("a");
        buf.insert("b");
        buf.undo();
        buf.insert("c");
        assert_eq!(buf.text(), "ac");
        assert!(!buf.redo(), "redo history was abandoned by the new edit");
    }

    #[test]
    fn undo_also_restores_a_deleted_selection() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.set_cursor(6, true);
        buf.delete_backward();
        assert_eq!(buf.text(), "world");
        assert!(buf.undo());
        assert_eq!(buf.text(), "hello world");
        assert_eq!(buf.selection(), Some(0..6));
    }

    #[test]
    fn move_word_right_from_mid_word_stops_at_the_word_end() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(2, false); // inside "hello"
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 5, "should stop at the end of the current word, not jump past it");
    }

    #[test]
    fn move_word_right_from_a_word_end_skips_whitespace_to_the_next_words_end() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(5, false); // right after "hello", before the space
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 11, "should skip the space and land at the end of \"world\"");
    }

    #[test]
    fn move_word_left_from_mid_word_stops_at_the_word_start() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(9, false); // inside "world"
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 6, "should stop at the start of \"world\"");
    }

    #[test]
    fn move_word_left_from_right_after_a_word_stays_at_that_words_start() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(5, false); // right after "hello"
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 0, "should land at the start of \"hello\", not skip past it");
    }

    #[test]
    fn repeated_word_moves_walk_through_every_word_in_a_sentence() {
        let mut buf = EditorBuffer::new("the quick fox");
        buf.set_cursor(0, false);
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 3, "end of \"the\"");
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 9, "end of \"quick\"");
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 13, "end of \"fox\", the last word");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 10, "start of \"fox\"");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 4, "start of \"quick\"");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 0, "start of \"the\"");
    }

    #[test]
    fn word_moves_treat_punctuation_as_its_own_word() {
        let mut buf = EditorBuffer::new("foo, bar");
        buf.set_cursor(0, false);
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 3, "stops at the end of \"foo\", before the comma");
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 4, "the comma is its own punctuation word");
    }

    #[test]
    fn shift_ctrl_word_right_extends_a_selection() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(0, false);
        buf.move_word_right(true);
        assert_eq!(buf.selection(), Some(0..5));
        buf.move_word_right(true);
        assert_eq!(buf.selection(), Some(0..11));
    }

    #[test]
    fn ctrl_word_left_without_shift_collapses_a_selection_to_its_start() {
        let mut buf = EditorBuffer::new("hello world");
        buf.set_cursor(2, false);
        buf.set_cursor(9, true);
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 2, "should collapse to the selection's start, like plain Left does");
        assert!(!buf.has_selection());
    }
}

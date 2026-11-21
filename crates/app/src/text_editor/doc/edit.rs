//! What an edit changed, in lines: the byte-level [`TextEdit`] callers report,
//! the [`EditSpan`] every classifier's patch walk starts from, and the
//! [`EditPatch`] handed back to the row cache.

use super::*;

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
pub(in crate::text_editor) struct TextEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
}

/// Where an edit falls in line terms — shared bookkeeping every per-
/// classifier patch function needs, computed once in [`DocumentCache::
/// record_edit`] rather than once per classifier.
pub(super) struct EditSpan {
    /// The old line index `edit.start_byte` falls on (also its new index:
    /// nothing before the edit moves).
    pub(super) start_line: usize,
    /// How many old lines the edit's old byte range spanned across (0 if
    /// it never crossed a newline at all).
    pub(super) lines_removed: usize,
    /// How many newlines the edit's new text contains.
    pub(super) lines_added: usize,
}

impl EditSpan {
    /// The last line (in the *new* numbering) whose text the edit itself
    /// changed. Reconvergence in the patch walks is only trustworthy past
    /// this line.
    pub(super) fn last_new_line(&self) -> usize {
        self.start_line + self.lines_added
    }

    pub(super) fn line_delta(&self) -> isize {
        self.lines_added as isize - self.lines_removed as isize
    }

    /// Where per-line arrays gain or lose entries so that everything after
    /// the edit lines up with its new line numbers: the entries before
    /// this point cover lines the walk rewrites anyway, the ones from it
    /// on are the untouched old tail.
    pub(super) fn splice_at(&self) -> usize {
        self.start_line + self.lines_removed.min(self.lines_added) + 1
    }
}

/// What [`DocumentCache::record_edit`] changed, for `RowHeightCache` to
/// mirror: resize at `splice_at` by `line_delta` lines, then re-derive
/// `touched`.
pub(in crate::text_editor) struct EditPatch {
    pub touched: Range<usize>,
    pub splice_at: usize,
    pub line_delta: isize,
}

impl EditPatch {
    /// The lines `touched` covered before the edit: later lines shifted by
    /// `line_delta`, so the old extent ends that much earlier. What a
    /// per-line table splices its replacement over.
    pub(in crate::text_editor) fn old_touched(&self) -> Range<usize> {
        let end = self.touched.end as isize - self.line_delta;
        assert!(end >= self.touched.start as isize);
        self.touched.start..end as usize
    }
}

impl DocumentCache {
    /// Resolves `edit`'s [`EditSpan`] against the *old* `line_index`, or
    /// `None` only for an empty cache (nothing to patch). Edits that change
    /// the line count are handled by [`Self::resize_lines`].
    pub(super) fn edit_span(&self, text: &str, edit: TextEdit) -> Option<EditSpan> {
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
}

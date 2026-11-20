//! The caret in side-by-side columns. A region's marker lines and the blank
//! line padding each marker take no rows there (the columns show only content
//! lines), so the caret must never sit on one: arrow keys step over them, going
//! down a column and then into the next in source order, and the keys that
//! would delete their newline do nothing. Ctrl+/ shows a region's source.

use gpui_kit::*;

use super::super::document_cache::DocumentCache;
use super::Editor;

/// The longest run of hidden lines a vertical move steps over: a close marker,
/// its padding, a separator and its two paddings, with room to spare.
const MAX_HIDDEN_RUN: usize = 8;

/// Whether `line` is in a side-by-side region without being a line of any
/// column: the caret cannot be there.
pub(in crate::text_editor) fn is_hidden_line(doc: &DocumentCache, line: usize) -> bool {
    doc.side_by_side(line)
        .is_some_and(|(region, _)| region.column_of(line).is_none())
}

impl Editor {
    /// Moves a caret on a hidden line to the nearest visible one in the
    /// direction it was heading (the start of the next, or the end of the
    /// previous), the other way when there is none.
    pub(in crate::text_editor) fn skip_hidden_lines(&mut self, forward: bool, extend: bool) {
        let target = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            if !is_hidden_line(&doc, line) {
                return;
            }
            let next = (line + 1..doc.line_count())
                .find(|&l| !is_hidden_line(&doc, l))
                .map(|l| doc.line_start_byte(l));
            let previous = (0..line)
                .rev()
                .find(|&l| !is_hidden_line(&doc, l))
                .map(|l| doc.line_end_byte(l));
            if forward {
                next.or(previous)
            } else {
                previous.or(next)
            }
        };
        if let Some(target) = target {
            self.buffer.set_cursor(target, extend);
        }
    }

    /// Up or Down, which keeps its column across the hidden lines it steps over.
    pub(in crate::text_editor) fn move_vertical(&mut self, down: bool, extend: bool) {
        for _ in 0..=MAX_HIDDEN_RUN {
            let before = self.buffer.cursor();
            if down {
                self.buffer.move_down(extend);
            } else {
                self.buffer.move_up(extend);
            }
            let line = self.document_cache.borrow().line_at(self.buffer.cursor());
            let hidden = is_hidden_line(&self.document_cache.borrow(), line);
            if !hidden || self.buffer.cursor() == before {
                break;
            }
        }
        self.skip_hidden_lines(down, extend);
    }

    /// Whether the newline `Backspace` (`backward`) or `Delete` would remove
    /// joins a column's edge line to a hidden one: that would break the
    /// region's blank-line padding, so the key does nothing there.
    pub(in crate::text_editor) fn at_region_edge(&self, backward: bool) -> bool {
        let doc = self.document_cache.borrow();
        let cursor = self.buffer.cursor();
        let line = doc.line_at(cursor);
        if backward {
            cursor == doc.line_start_byte(line) && line > 0 && is_hidden_line(&doc, line - 1)
        } else {
            cursor == doc.line_end_byte(line)
                && line + 1 < doc.line_count()
                && is_hidden_line(&doc, line + 1)
        }
    }

    /// Works out which region is shown as source from where the caret is. Called
    /// before every render.
    pub(in crate::text_editor) fn sync_column_view(&mut self, window: &Window) {
        let focused = self.focus_handle.is_focused(window);
        let raw = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            let region = focused.then(|| doc.region_index_at(line)).flatten();
            self.raw_region_pinned.filter(|&r| Some(r) == region)
        };
        let changed = self.document_cache.borrow_mut().set_raw_region(raw);
        if changed {
            self.row_height_cache.borrow_mut().invalidate_width();
        }
    }

    /// Ctrl+/ with the caret in a region: shows its source instead of its
    /// columns, or the columns again. `false` when the caret is in none.
    pub(in crate::text_editor) fn toggle_raw_region(&mut self, cx: &mut Context<Self>) -> bool {
        let (index, raw) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            let Some(index) = doc.region_index_at(line) else {
                return false;
            };
            (index, doc.raw_region() == Some(index))
        };
        self.raw_region_pinned = (!raw).then_some(index);
        cx.notify();
        true
    }
}

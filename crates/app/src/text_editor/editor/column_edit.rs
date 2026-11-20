//! The caret in side-by-side columns. A region's marker lines and the blank
//! line padding each marker take no rows there (the columns show only content
//! lines), so the caret must never sit on one: arrow keys step over them, going
//! down a column and then into the next in source order, and the keys that
//! would delete their newline do nothing. Ctrl+/ shows a region's source.

use gpui_kit::*;

use super::super::block_preview::LineRole;
use super::super::column_ops;
use super::super::columns::Columns;
use super::super::columns::Kind;
use super::super::document_cache::DocumentCache;
use super::Editor;

/// What the right-click menu offers about the structure around the caret.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::text_editor) enum ColumnMenu {
    /// The caret is in a region: add a column (unless at the limit) or unwrap.
    Region { can_add: bool },
    /// The caret is in an aside: flip it to the other side, or unwrap.
    Aside,
    /// The caret is in a plain block that can become the first column, or a float.
    Block,
}

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
        // A divider being dragged lays the region out at the shares it is on.
        let preview = self
            .column_drag
            .as_ref()
            .map(|d| (d.region, d.percent.clone()));
        let mut doc = self.document_cache.borrow_mut();
        let changed = doc.set_raw_region(raw) | doc.set_column_preview(preview);
        drop(doc);
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

impl Editor {
    /// The region and block-wrapping entries the menu shows for the caret.
    pub(in crate::text_editor) fn column_menu_state(&self) -> Option<ColumnMenu> {
        let doc = self.document_cache.borrow();
        let line = doc.line_at(self.buffer.cursor());
        if let Some(i) = doc.region_index_at(line) {
            let region = doc.columns()[i];
            if region.is_aside() {
                return Some(ColumnMenu::Aside);
            }
            return Some(ColumnMenu::Region {
                can_add: region.count < crate::text_editor::columns::MAX_COLUMNS,
            });
        }
        self.block_lines(&doc, line).map(|_| ColumnMenu::Block)
    }

    /// The lines of the paragraph-like block holding `line`: the run of
    /// non-blank lines around it, with a blank line (or the document edge) on
    /// both sides. `None` in code, a table or a callout, whose own structure
    /// a column marker would cut.
    fn block_lines(&self, doc: &DocumentCache, line: usize) -> Option<std::ops::Range<usize>> {
        let text = self.buffer.text();
        let blank = |l: usize| {
            doc.line_end_byte(l) == doc.line_start_byte(l)
                || text[doc.line_start_byte(l)..doc.line_end_byte(l)]
                    .trim()
                    .is_empty()
        };
        let plain = |l: usize| {
            doc.line_roles().get(l) == Some(&LineRole::Plain)
                && crate::text_editor::table_preview::table_index_at(doc.tables(), l).is_none()
                && crate::text_editor::callout::callout_index_at(doc.callouts(), l).is_none()
        };
        if blank(line) || !plain(line) {
            return None;
        }
        let mut start = line;
        while start > 0 && !blank(start - 1) {
            start -= 1;
        }
        let mut end = line + 1;
        while end < doc.line_count() && !blank(end) {
            end += 1;
        }
        (start..end).all(plain).then_some(start..end)
    }

    /// Replaces region `index`'s lines with what `make` returns, as one undo
    /// step, and leaves the caret on the region's first visible line.
    fn edit_region(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        make: impl FnOnce(&[&str], &Columns) -> Option<Vec<String>>,
    ) -> bool {
        let (range, text) = {
            let doc = self.document_cache.borrow();
            let Some(&region) = doc.columns().get(index) else {
                return false;
            };
            let source = self.buffer.text();
            let old: Vec<&str> = (region.start_line..region.end_line)
                .map(|l| &source[doc.line_start_byte(l)..doc.line_end_byte(l)])
                .collect();
            let Some(new) = make(&old, &region) else {
                return false;
            };
            let range =
                doc.line_start_byte(region.start_line)..doc.line_end_byte(region.end_line - 1);
            (range, new.join("\n"))
        };
        // A move first, so this edit never merges into the typing before it.
        self.buffer.set_cursor(range.start, false);
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.buffer.set_cursor(range.start, false);
        self.skip_hidden_lines(true, false);
        cx.notify();
        true
    }

    /// Sets region `index`'s shares, after a divider drag.
    pub(in crate::text_editor) fn set_region_ratio(
        &mut self,
        index: usize,
        percent: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.edit_region(index, window, cx, |lines, region| {
            if percent.len() != region.count || percent == region.percent[..region.count] {
                return None;
            }
            Some(match region.kind {
                Kind::Aside => column_ops::aside_markers(lines, region, &percent, false),
                Kind::Columns => column_ops::with_ratio(lines, &percent),
            })
        })
    }

    /// Adds an empty column at the right of region `index`.
    pub(in crate::text_editor) fn add_region_column(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.edit_region(index, window, cx, |lines, region| {
            if region.is_aside() {
                column_ops::with_added_float(lines, region)
            } else {
                column_ops::with_added_column(lines, region)
            }
        })
    }

    /// Turns region `index` back into its columns' content, one after another.
    pub(in crate::text_editor) fn unwrap_region(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.edit_region(index, window, cx, |lines, region| {
            Some(column_ops::unwrapped(lines, region))
        })
    }

    /// Flips the aside holding the caret to its other side.
    pub(in crate::text_editor) fn flip_aside(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let index = {
            let doc = self.document_cache.borrow();
            doc.region_index_at(doc.line_at(self.buffer.cursor()))
        };
        let Some(index) = index else {
            return false;
        };
        self.edit_region(index, window, cx, |lines, region| {
            region
                .is_aside()
                .then(|| column_ops::flipped(lines, region))
        })
    }

    /// Floats the block around the caret to one side, the paragraphs right
    /// after it (up to three) flowing around it.
    pub(in crate::text_editor) fn make_float(
        &mut self,
        right: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (range, text) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            let Some(block) = self.block_lines(&doc, line) else {
                return false;
            };
            let source = self.buffer.text();
            let line_text = |l: usize| &source[doc.line_start_byte(l)..doc.line_end_byte(l)];
            let mut end = block.end;
            let mut text: Vec<&str> = Vec::new();
            for _ in 0..3 {
                let Some(next) = (end..doc.line_count()).find(|&l| !line_text(l).trim().is_empty())
                else {
                    break;
                };
                let Some(following) = self.block_lines(&doc, next) else {
                    break;
                };
                if !text.is_empty() {
                    text.push("");
                }
                text.extend(following.clone().map(line_text));
                end = following.end;
            }
            let lines: Vec<&str> = block.clone().map(line_text).collect();
            let range = doc.line_start_byte(block.start)..doc.line_end_byte(end - 1);
            (range, column_ops::floated(&lines, &text, right).join("\n"))
        };
        self.buffer.set_cursor(range.start, false);
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.buffer
            .set_cursor(range.start + "::: aside right 40%\n\n".len(), false);
        cx.notify();
        true
    }

    /// The menu action for the caret's region (`add`) or unwrap.
    pub(in crate::text_editor) fn run_region_menu(
        &mut self,
        add: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let index = {
            let doc = self.document_cache.borrow();
            doc.region_index_at(doc.line_at(self.buffer.cursor()))
        };
        let Some(index) = index else {
            return false;
        };
        if add {
            self.add_region_column(index, window, cx)
        } else {
            self.unwrap_region(index, window, cx)
        }
    }

    /// Makes the block around the caret the first of two columns.
    pub(in crate::text_editor) fn make_columns(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (range, text) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            let Some(block) = self.block_lines(&doc, line) else {
                return false;
            };
            let source = self.buffer.text();
            let lines: Vec<&str> = block
                .clone()
                .map(|l| &source[doc.line_start_byte(l)..doc.line_end_byte(l)])
                .collect();
            let range = doc.line_start_byte(block.start)..doc.line_end_byte(block.end - 1);
            (range, column_ops::wrapped(&lines).join("\n"))
        };
        self.buffer.set_cursor(range.start, false);
        self.replace_in_buffer(range.clone(), &text, window, cx);
        // The new region's first column, which holds the block's first line.
        let first = range.start + "::: columns\n\n".len();
        self.buffer.set_cursor(first, false);
        cx.notify();
        true
    }
}

impl Editor {
    /// The end of a divider drag: applies the shares it ended on.
    pub(in crate::text_editor) fn finish_column_drag(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.column_drag.take() else {
            return;
        };
        self.set_region_ratio(drag.region, drag.percent, window, cx);
        cx.notify();
    }
}

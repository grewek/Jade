//! The caret in side-by-side columns. A region's marker lines and the blank
//! line padding each marker take no rows there (the columns show only content
//! lines), so the caret must never sit on one: arrow keys step over them, going
//! down a column and then into the next in source order, and the keys that
//! would delete their newline do nothing. Ctrl+/ shows a region's source.

use std::ops::Range;

use gpui_kit::*;

use super::super::columns;
use super::super::columns::Columns;
use super::super::columns::Kind;
use super::super::doc::{DocumentCache, TextEdit};
use super::super::element::render_params;
use super::super::preview::LineRole;
use super::super::rows::{Change, Inputs};
use super::Editor;

/// What the right-click menu offers about the structure around the caret.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::text_editor) enum ColumnMenu {
    /// The caret is in a region: add a column (unless at the limit) or unwrap.
    Region { can_add: bool },
    /// The caret is in an aside: flip it to the other side, or unwrap.
    Aside,
    /// The caret is in a float block's region: flip the float, move it a
    /// paragraph up or down, or unwrap it.
    Float,
    /// The caret is in a plain block that can become the first column, or a float.
    Block,
    /// The caret is on a blank line where an empty region can be put.
    Blank,
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

/// Where `offset`, outside `replaced`, is once that range holds `len` bytes.
fn shifted(offset: usize, replaced: &Range<usize>, len: usize) -> usize {
    if offset < replaced.start {
        offset
    } else {
        offset - replaced.len() + len
    }
}

/// Whether `line` is a marker of a region laid out side by side.
fn is_shown_marker(doc: &DocumentCache, line: usize) -> bool {
    doc.side_by_side(line)
        .is_some_and(|(region, _)| region.is_marker(line))
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

    /// The caret where the mouse points. A point below a region that ends the
    /// note is on its close marker, where typing would end the region: the
    /// caret stops at the end of the text before it.
    pub(in crate::text_editor) fn place_from_pointer(&mut self, offset: usize, extend: bool) {
        self.buffer.set_cursor(offset, extend);
        self.skip_hidden_lines(false, extend);
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

    /// Inserts `newlines` at `at`, the note's end (`down`) or start, and puts
    /// the caret on the outermost new line.
    pub(in crate::text_editor) fn add_line_outside(
        &mut self,
        (at, newlines): (usize, String),
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caret = if down { at + newlines.len() } else { at };
        self.buffer.end_undo_group();
        self.replace_in_buffer(at..at, &newlines, window, cx);
        self.buffer.set_cursor(caret, false);
    }

    /// The newlines that typing `text` over `range` must bring along on each
    /// side, so a marker next to the caret's line stays alone between blank
    /// lines and its region survives. In a column (only one with nothing but
    /// padding has a line next to a marker) the padding always comes, to keep
    /// the caret off it; on the blank line outside a region, only once that
    /// line would no longer be blank.
    pub(in crate::text_editor) fn column_padding(
        &self,
        range: &Range<usize>,
        text: &str,
    ) -> (&'static str, &'static str) {
        let doc = self.document_cache.borrow();
        let (first, last) = (doc.line_at(range.start), doc.line_at(range.end));
        let column = match doc.side_by_side(first) {
            Some((region, _)) => match region.column_of(first) {
                Some(k) if region.column_of(last) == Some(k) => true,
                _ => return ("", ""),
            },
            None => false,
        };
        let lead = first > 0 && is_shown_marker(&doc, first - 1);
        let trail = last + 1 < doc.line_count() && is_shown_marker(&doc, last + 1);
        if !lead && !trail {
            return ("", "");
        }
        // What the first and the last line of the result hold.
        let source = self.buffer.text();
        let blank = |s: &str| s.trim().is_empty();
        let before = blank(&source[doc.line_start_byte(first)..range.start]);
        let after = blank(&source[range.end..doc.line_end_byte(last)]);
        let one_line = !text.contains('\n');
        let mut pieces = text.split('\n');
        let first_blank = before && pieces.next().is_none_or(blank) && (!one_line || after);
        let last_blank =
            after && text.rsplit('\n').next().is_none_or(blank) && (!one_line || before);
        let pad = |needed: bool| if needed { "\n" } else { "" };
        (
            pad(lead && (column || !first_blank)),
            pad(trail && (column || !last_blank)),
        )
    }

    /// Typing, pasting, Enter or Tab on a line that needs [`Self::column_padding`]:
    /// the text and its padding as one edit, the caret after the text. `false`
    /// (nothing done) everywhere else.
    pub(in crate::text_editor) fn insert_in_bare_column(
        &mut self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (lead, trail) = self.column_padding(&range, text);
        if text.is_empty() || (lead.is_empty() && trail.is_empty()) {
            return false;
        }
        let caret = range.start + lead.len() + text.len();
        let padded = [lead, text, trail].concat();
        let edit = TextEdit {
            start_byte: range.start,
            old_end_byte: range.end,
            new_end_byte: range.start + padded.len(),
        };
        self.buffer.replace_range_with(range, &padded);
        self.buffer.set_cursor(caret, false);
        self.record_edit(edit, window, cx);
        true
    }

    /// Whether `Backspace` (`backward`) or `Delete` at a line's edge would
    /// break a region, and so does nothing: it would join a column's edge line
    /// to a hidden one, or put text right next to a marker.
    pub(in crate::text_editor) fn at_region_edge(&self, backward: bool) -> bool {
        let doc = self.document_cache.borrow();
        let cursor = self.buffer.cursor();
        let line = doc.line_at(cursor);
        // The newline in question joins lines `above` and `above + 1`.
        let above = if backward {
            if cursor != doc.line_start_byte(line) || line == 0 {
                return false;
            }
            line - 1
        } else {
            if cursor != doc.line_end_byte(line) || line + 1 >= doc.line_count() {
                return false;
            }
            line
        };
        let below = above + 1;
        if is_hidden_line(&doc, above) || is_hidden_line(&doc, below) {
            return true;
        }
        let next_to_marker = (above > 0 && is_shown_marker(&doc, above - 1))
            || (below + 1 < doc.line_count() && is_shown_marker(&doc, below + 1));
        next_to_marker && !(self.is_blank_line(&doc, above) && self.is_blank_line(&doc, below))
    }

    /// Works out which region is shown as source from where the caret is. Called
    /// before every render.
    pub(in crate::text_editor) fn sync_column_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let before = (doc.view.raw_region(), doc.view.column_preview_region());
        let changed = doc.view.set_raw_region(raw) | doc.view.set_column_preview(preview);
        let after = (doc.view.raw_region(), doc.view.column_preview_region());
        let regions: Vec<usize> = [before.0, before.1, after.0, after.1]
            .into_iter()
            .flatten()
            .collect();
        drop(doc);
        if changed {
            self.remeasure_regions(&regions, window, cx);
        }
    }

    /// Re-measures the rows of `regions` alone. Only those regions' heights
    /// changed, so a full rebuild would re-seed every line outside the
    /// viewport with estimates and move the document under the scroll
    /// position (a divider drag then jumped elsewhere).
    fn remeasure_regions(
        &mut self,
        regions: &[usize],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = render_params(self, window, cx);
        let inp = Inputs {
            text: self.buffer.text(),
            cursor: self.buffer.cursor(),
            focused: self.focus_handle.is_focused(window),
            width: params.available_width,
            font: &params.font,
            font_size: params.font_size,
            text_color: params.text_color,
        };
        let mut rows = self.row_height_cache.borrow_mut();
        for &index in regions {
            let Some(region) = self.document_cache.borrow().columns().get(index).copied() else {
                continue;
            };
            let lines = region.start_line..region.end_line;
            rows.update(&self.document_cache, Change::Lines(lines), &inp, window);
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
            (index, doc.view.raw_region() == Some(index))
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
            if region.tail {
                return Some(ColumnMenu::Float);
            }
            if region.is_aside() {
                return Some(ColumnMenu::Aside);
            }
            return Some(ColumnMenu::Region {
                can_add: region.count < crate::text_editor::columns::MAX_COLUMNS,
            });
        }
        if self.block_lines(&doc, line).is_some() {
            return Some(ColumnMenu::Block);
        }
        // Not a blank line of fenced code: a marker there would be code.
        (self.is_blank_line(&doc, line) && doc.line_roles().get(line) == Some(&LineRole::Plain))
            .then_some(ColumnMenu::Blank)
    }

    fn is_blank_line(&self, doc: &DocumentCache, line: usize) -> bool {
        let range = doc.line_start_byte(line)..doc.line_end_byte(line);
        self.buffer.text()[range].trim().is_empty()
    }

    /// The lines of the paragraph-like block holding `line`: the run of
    /// non-blank lines around it, with a blank line (or the document edge) on
    /// both sides. `None` in code, a table or a callout, whose own structure
    /// a column marker would cut.
    fn block_lines(&self, doc: &DocumentCache, line: usize) -> Option<Range<usize>> {
        let blank = |l: usize| self.is_blank_line(doc, l);
        let plain = |l: usize| {
            doc.line_roles().get(l) == Some(&LineRole::Plain)
                && crate::text_editor::table::table_index_at(doc.tables(), l).is_none()
                && crate::text_editor::preview::callout::callout_index_at(doc.callouts(), l)
                    .is_none()
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
    /// step. The caret stays with the text it was in.
    fn edit_region(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        make: impl FnOnce(&[&str], &Columns) -> Option<Vec<String>>,
    ) -> bool {
        let caret = self.buffer.cursor();
        let (range, text, caret) = {
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
            let whole =
                doc.line_start_byte(region.start_line)..doc.line_end_byte(region.end_line - 1);
            let new_len = new.iter().map(|l| l.len() + 1).sum::<usize>() - 1;
            let caret = if caret < whole.start || caret > whole.end {
                shifted(caret, &whole, new_len)
            } else {
                let line = doc.line_at(caret);
                let i = line - region.start_line;
                let j = columns::ops::moved_line(&old, &new, &region, i);
                let before: usize = new[..j].iter().map(|l| l.len() + 1).sum();
                whole.start + before + (caret - doc.line_start_byte(line)).min(new[j].len())
            };
            // The same lines but a few (a share changed): replace just those,
            // not a float block's whole text.
            let same = |i: usize| new.get(i).is_some_and(|n| *n == old[i]);
            let (first, last) = if new.len() == old.len() {
                let first = (0..old.len()).find(|&i| !same(i));
                let last = (0..old.len()).rev().find(|&i| !same(i));
                match first.zip(last) {
                    Some(span) => span,
                    None => return false,
                }
            } else {
                (0, old.len() - 1)
            };
            let range = doc.line_start_byte(region.start_line + first)
                ..doc.line_end_byte(region.start_line + last);
            let new_span = if new.len() == old.len() {
                first..=last
            } else {
                0..=new.len() - 1
            };
            (range, new[new_span].join("\n"), caret)
        };
        // So this edit never merges into the typing before it.
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.place_caret(caret);
        cx.notify();
        true
    }

    /// The caret after a structure edit, never left on a hidden line (typing
    /// there would end the region).
    fn place_caret(&mut self, caret: usize) {
        self.buffer.set_cursor(caret, false);
        self.skip_hidden_lines(true, false);
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
                Kind::Aside if region.tail => columns::ops::float_markers(lines, region, &percent),
                Kind::Aside => columns::ops::aside_markers(lines, region, &percent, false),
                Kind::Columns => columns::ops::with_ratio(lines, &percent),
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
                columns::ops::with_added_float(lines, region)
            } else {
                columns::ops::with_added_column(lines, region)
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
            Some(columns::ops::unwrapped(lines, region))
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
                .then(|| columns::ops::flipped(lines, region))
        })
    }

    /// The float of the caret's float-block region the menu acts on: the last
    /// one opened at or before the caret's line (the float its text flows
    /// around), as `(region index, float index)`.
    fn float_at_caret(&self) -> Option<(usize, usize)> {
        let doc = self.document_cache.borrow();
        let line = doc.line_at(self.buffer.cursor());
        let index = doc.region_index_at(line)?;
        let region = doc.columns()[index];
        if !region.tail {
            return None;
        }
        let k = (0..region.count - 1)
            .rev()
            .find(|&k| region.open_of(k) <= line)
            .unwrap_or(0);
        Some((index, k))
    }

    /// Flips the float the caret belongs to (a float block) to its other side.
    pub(in crate::text_editor) fn flip_float(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((index, k)) = self.float_at_caret() else {
            return false;
        };
        self.edit_region(index, window, cx, |lines, region| {
            Some(columns::ops::float_flipped(lines, region, k))
        })
    }

    /// Dissolves the float block the caret belongs to into plain paragraphs.
    pub(in crate::text_editor) fn unwrap_float(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((index, k)) = self.float_at_caret() else {
            return false;
        };
        let caret = self.buffer.cursor();
        let (range, text, caret) = {
            let doc = self.document_cache.borrow();
            let region = doc.columns()[index];
            let source = self.buffer.text();
            let content: Vec<&str> = region
                .column_lines(k)
                .map(|l| &source[doc.line_start_byte(l)..doc.line_end_byte(l)])
                .collect();
            let range =
                doc.line_start_byte(region.open_of(k))..doc.line_end_byte(region.separators()[k]);
            let text = content.join("\n");
            let first = doc.line_start_byte(region.column_lines(k).start);
            let caret = if caret < range.start || caret > range.end {
                shifted(caret, &range, text.len())
            } else {
                range.start + caret.saturating_sub(first).min(text.len())
            };
            (range, text, caret)
        };
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.place_caret(caret);
        cx.notify();
        true
    }

    /// Moves the float block the caret belongs to past the paragraph after it
    /// (`down`) or before it, so the text it flows around changes. `false`
    /// when there is no plain paragraph that way.
    pub(in crate::text_editor) fn move_float(
        &mut self,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((index, k)) = self.float_at_caret() else {
            return false;
        };
        let caret = self.buffer.cursor();
        let (range, text, caret) = {
            let doc = self.document_cache.borrow();
            let region = doc.columns()[index];
            let source = self.buffer.text();
            let line_text = |l: usize| &source[doc.line_start_byte(l)..doc.line_end_byte(l)];
            let blank = |l: usize| line_text(l).trim().is_empty();
            let (open, close) = (region.open_of(k), region.separators()[k]);
            // The paragraph on the move's side, and no marker of any region.
            let target = if down {
                (close + 1..doc.line_count()).find(|&l| !blank(l))
            } else {
                (0..open).rev().find(|&l| !blank(l))
            };
            let Some(target) = target else {
                return false;
            };
            let Some(para) = self.block_lines(&doc, target) else {
                return false;
            };
            if para
                .clone()
                .any(|l| doc.columns().iter().any(|r| r.is_marker(l)))
            {
                return false;
            }
            let block: Vec<&str> = (open..=close).map(line_text).collect();
            let para_text: Vec<&str> = para.clone().map(line_text).collect();
            let (span, first, second) = if down {
                (open..para.end, &para_text, &block)
            } else {
                (para.start..close + 1, &block, &para_text)
            };
            let mut lines: Vec<&str> = first.clone();
            lines.push("");
            lines.extend(second.iter().copied());
            let range = doc.line_start_byte(span.start)..doc.line_end_byte(span.end - 1);
            let text = lines.join("\n");
            // The caret goes with whichever of the two blocks it was in; the
            // second one starts after the first and a blank line.
            let second_start = range.start + first.iter().map(|l| l.len() + 1).sum::<usize>() + 1;
            let (block_start, para_start) = if down {
                (second_start, range.start)
            } else {
                (range.start, second_start)
            };
            let old_block = doc.line_start_byte(open)..doc.line_end_byte(close);
            let old_para = doc.line_start_byte(para.start)..doc.line_end_byte(para.end - 1);
            let caret = if caret < range.start || caret > range.end {
                shifted(caret, &range, text.len())
            } else if (old_para.start..=old_para.end).contains(&caret) {
                para_start + (caret - old_para.start)
            } else {
                block_start + (caret.clamp(old_block.start, old_block.end) - old_block.start)
            };
            (range, text, caret)
        };
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.place_caret(caret);
        cx.notify();
        true
    }

    /// Floats the block around the caret to one side, the paragraphs right
    /// after it (up to three) flowing around it.
    pub(in crate::text_editor) fn make_float(
        &mut self,
        right: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (range, text, first) = {
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
            let out = columns::ops::floated(&lines, &text, right);
            // Past the open marker and its padding line.
            let first = range.start + out[0].len() + "\n\n".len();
            (range, out.join("\n"), first)
        };
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.buffer.set_cursor(first, false);
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

    /// Puts an empty region (`lines`, as `columns::ops` makes it) on the
    /// caret's blank line, with the blank lines its markers need around it,
    /// and the caret in its first part.
    fn insert_region(
        &mut self,
        lines: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (range, lead, trail) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            if !self.is_blank_line(&doc, line) {
                return false;
            }
            let lead = line > 0 && !self.is_blank_line(&doc, line - 1);
            let trail = line + 1 < doc.line_count() && !self.is_blank_line(&doc, line + 1);
            let range = doc.line_start_byte(line)..doc.line_end_byte(line);
            (range, lead, trail)
        };
        let pad = |needed: bool| if needed { "\n" } else { "" };
        let text = [pad(lead), &lines.join("\n"), pad(trail)].concat();
        // Past the open marker and its padding line.
        let first = range.start + pad(lead).len() + lines[0].len() + "\n\n".len();
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        self.buffer.set_cursor(first, false);
        cx.notify();
        true
    }

    /// Two empty columns on the caret's blank line.
    pub(in crate::text_editor) fn insert_columns(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.insert_region(columns::ops::wrapped(&[""]), window, cx)
    }

    /// An empty float and the empty text beside it, on the caret's blank line.
    pub(in crate::text_editor) fn insert_float(
        &mut self,
        right: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.insert_region(columns::ops::floated(&[""], &[], right), window, cx)
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
            (range, columns::ops::wrapped(&lines).join("\n"))
        };
        self.buffer.end_undo_group();
        self.replace_in_buffer(range.clone(), &text, window, cx);
        // The new region's first column, which holds the block's first line.
        // Past the open marker and the blank line after it.
        let first = range.start + text.find("\n\n").map_or(0, |i| i + 2);
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

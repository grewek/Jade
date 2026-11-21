//! Editing a table's cells in place. The cell holding the caret shows its text
//! as typed and every other cell stays rendered; keys move between cells and
//! never cross a cell's border, so a `|` of the source cannot be edited away.
//! The source stays ragged: only the touched cell's text changes.

use crate::text_editor::table;
use std::borrow::Cow;
use std::ops::Range;

use gpui_kit::*;

use super::super::doc::TextEdit;
use super::super::element::{TableColumns, render_params};
use super::super::rows::Invalidate;
use super::super::table::handles::{Handle, TableDrag};
use super::super::table::ops::Op;
use super::super::table::{Align, Table, table_index_at};
use super::super::table::{cell_at, cell_ranges, edit_range};
use super::Editor;

/// A cell of the table being edited, in absolute buffer bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cell {
    pub table: usize,
    pub line: usize,
    pub column: usize,
    pub line_start: usize,
    /// The editable text of the cell: `lo..hi`.
    pub lo: usize,
    pub hi: usize,
}

/// The row below `line` in `table`, skipping the delimiter row.
fn row_after(table: &Table, line: usize) -> Option<usize> {
    let next = if line == table.start_line {
        line + 2
    } else {
        line + 1
    };
    (next < table.end_line).then_some(next)
}

/// The row above `line` in `table`, skipping the delimiter row.
fn row_before(table: &Table, line: usize) -> Option<usize> {
    match line.checked_sub(table.start_line + 2) {
        Some(0) => Some(table.start_line),
        Some(_) => Some(line - 1),
        None => None,
    }
}

/// `text` as it goes into a cell: a line break becomes `<br>` and a `|` is
/// escaped, since either would otherwise end the cell or the row.
pub(super) fn escape_for_cell(text: &str) -> Cow<'_, str> {
    if !text.contains(['|', '\n', '\r', '\t']) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '|' => out.push_str("\\|"),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' | '\n' => out.push_str("<br>"),
            '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A new row of `columns` empty cells, `| | |`.
fn empty_row(columns: usize) -> String {
    let mut row = String::from("|");
    for _ in 0..columns {
        row.push_str(" |");
    }
    row
}

impl Editor {
    /// The cell under the caret while it is being edited.
    pub(super) fn editing_cell(&self) -> Option<Cell> {
        let column = self.document_cache.borrow().view.editing_cell()?;
        self.cell_at_line(column.0, column.1)
    }

    /// Cell `column` (the last one if the row has fewer) of `line`.
    fn cell_at_line(&self, line: usize, column: usize) -> Option<Cell> {
        let doc = self.document_cache.borrow();
        let table = table_index_at(doc.tables(), line)?;
        let text = self.buffer.text();
        let start = doc.line_start_byte(line);
        let content = &text[start..doc.line_end_byte(line)];
        let ranges = cell_ranges(content);
        let column = column.min(ranges.len().checked_sub(1)?);
        let range = edit_range(content, ranges[column].clone());
        Some(Cell {
            table,
            line,
            column,
            line_start: start,
            lo: start + range.start,
            hi: start + range.end,
        })
    }

    /// Works out which table is raw and which cell is edited from where the
    /// caret is, and moves a caret that sits on a pipe or its padding into the
    /// cell. Called before every render.
    pub(super) fn sync_table_view(&mut self, window: &Window) {
        let focused = self.focus_handle.is_focused(window);
        let cursor = self.buffer.cursor();
        let (raw, editing) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(cursor);
            let table = focused
                .then(|| table_index_at(doc.tables(), line))
                .flatten();
            let mut raw = self.raw_pinned.filter(|&r| Some(r) == table);
            let mut editing = None;
            if let Some(index) = table {
                // The delimiter row has no cells to edit, so it is shown as source.
                if doc.tables()[index].is_delimiter(line) {
                    raw = Some(index);
                }
                if raw != Some(index) {
                    let content_start = doc.line_start_byte(line);
                    let content = &self.buffer.text()[content_start..doc.line_end_byte(line)];
                    editing = Some((line, cell_at(content, cursor - content_start)));
                }
            }
            (raw, editing)
        };
        let editing = editing.and_then(|(line, column)| {
            let cell = self.cell_at_line(line, column)?;
            // A selection reaching outside the cell is not an edit of it.
            let inside = |at: usize| (cell.lo..=cell.hi).contains(&at);
            match self.buffer.selection() {
                Some(selection) if !(inside(selection.start) && inside(selection.end)) => None,
                Some(_) => Some(cell),
                None => {
                    let text = self.buffer.text();
                    // Not inside a `<br>` either: it is one step, shown as a break.
                    let inside_tag = (1..4).find(|&k| {
                        cursor >= k
                            && text
                                .get(cursor - k..)
                                .is_some_and(|t| t.starts_with("<br>"))
                    });
                    let wanted = match inside_tag {
                        Some(k) => cursor - k + 4,
                        None => cursor.clamp(cell.lo, cell.hi),
                    };
                    if wanted != cursor {
                        self.buffer.set_cursor(wanted, false);
                    }
                    Some(cell)
                }
            }
        });
        let changed = self
            .document_cache
            .borrow_mut()
            .view
            .set_table_view(raw, editing.map(|c| (c.line, c.column)));
        if changed {
            self.row_height_cache
                .borrow_mut()
                .invalidate(Invalidate::Width);
        }
    }

    /// Ctrl+/: shows the source of the table with the caret instead of its box,
    /// or the box again. `false` when the caret is in no table.
    pub(super) fn toggle_raw_table(&mut self, cx: &mut Context<Self>) -> bool {
        let (index, raw, leave) = {
            let doc = self.document_cache.borrow();
            let line = doc.line_at(self.buffer.cursor());
            let Some(index) = table_index_at(doc.tables(), line) else {
                return false;
            };
            let table = &doc.tables()[index];
            // The delimiter row is always raw, so the caret leaves it.
            let leave = table
                .is_delimiter(line)
                .then(|| row_after(table, table.start_line))
                .flatten();
            (index, doc.view.raw_table() == Some(index), leave)
        };
        self.raw_pinned = (!raw).then_some(index);
        if let Some(cell) = leave.and_then(|line| self.cell_at_line(line, 0)) {
            self.buffer.set_cursor(cell.lo, false);
        }
        cx.notify();
        true
    }

    /// Replaces `range` with `text`, as one edit.
    pub(super) fn replace_in_buffer(
        &mut self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = TextEdit {
            start_byte: range.start,
            old_end_byte: range.end,
            new_end_byte: range.start + text.len(),
        };
        self.buffer.replace_range_with(range, text);
        self.record_edit(edit, window, cx);
    }

    /// What typing or pasting `text` over `range` does while a cell is edited:
    /// the range kept inside the cell and the text escaped. `None` outside one.
    pub(in crate::text_editor) fn cell_input(
        &self,
        range: Range<usize>,
        text: &str,
    ) -> Option<(Range<usize>, String)> {
        let cell = self.editing_cell()?;
        let range = range.start.clamp(cell.lo, cell.hi)..range.end.clamp(cell.lo, cell.hi);
        Some((range, escape_for_cell(text).into_owned()))
    }

    /// Keeps a mouse drag's end inside the cell being edited.
    pub(in crate::text_editor) fn clamp_to_cell(&self, offset: usize) -> usize {
        self.editing_cell()
            .map_or(offset, |cell| offset.clamp(cell.lo, cell.hi))
    }

    /// Ctrl/Cmd+A: the cell's text, not the document's.
    pub(super) fn select_cell(&mut self) -> bool {
        let Some(cell) = self.editing_cell() else {
            return false;
        };
        self.buffer.set_cursor(cell.lo, false);
        self.buffer.set_cursor(cell.hi, true);
        true
    }

    /// Handles `key` when the caret is in a cell; `false` leaves it to the
    /// ordinary editor (typing within a cell needs nothing special).
    pub(super) fn table_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cell) = self.editing_cell() else {
            return false;
        };
        let cursor = self.buffer.cursor();
        let selected = self.buffer.has_selection();
        let handled = match key {
            // A `<br>` shows as a line break, so the caret steps over it whole.
            "left" if self.buffer.text()[..cursor].ends_with("<br>") => {
                self.buffer.set_cursor(cursor - 4, shift);
                true
            }
            "right" if self.buffer.text()[cursor..].starts_with("<br>") => {
                self.buffer.set_cursor(cursor + 4, shift);
                true
            }
            "left" if cursor <= cell.lo && !selected => {
                if !shift {
                    self.move_to_neighbour(&cell, false, false);
                }
                true
            }
            "right" if cursor >= cell.hi && !selected => {
                if !shift {
                    self.move_to_neighbour(&cell, true, false);
                }
                true
            }
            "up" | "down" => {
                self.move_vertically(&cell, key == "down", shift, window, cx);
                true
            }
            "home" => {
                self.buffer.set_cursor(cell.lo, shift);
                true
            }
            "end" => {
                self.buffer.set_cursor(cell.hi, shift);
                true
            }
            "backspace" | "delete" if !selected => {
                return self.delete_next_to_caret(&cell, key == "delete", window, cx);
            }
            "enter" if shift => {
                let range = self.buffer.selection().unwrap_or(cursor..cursor);
                self.replace_in_buffer(range, "<br>", window, cx);
                true
            }
            "enter" => {
                self.enter_in_cell(&cell, window, cx);
                true
            }
            "escape" => {
                self.leave_table(&cell, window, cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.notify();
        }
        handled
    }

    /// Backspace or Delete with no selection. A `\\|` or `<br>` goes whole, and
    /// nothing goes past the cell's border; any other character is left to the
    /// ordinary editor (`false`).
    fn delete_next_to_caret(
        &mut self,
        cell: &Cell,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let cursor = self.buffer.cursor();
        if cursor == if forward { cell.hi } else { cell.lo } {
            return true;
        }
        let text = self.buffer.text();
        let token = ["\\|", "<br>"].into_iter().find(|token| {
            if forward {
                text[cursor..cell.hi].starts_with(token)
            } else {
                text[cell.lo..cursor].ends_with(token)
            }
        });
        let Some(token) = token else {
            return false;
        };
        let range = if forward {
            cursor..cursor + token.len()
        } else {
            cursor - token.len()..cursor
        };
        self.replace_in_buffer(range, "", window, cx);
        cx.notify();
        true
    }

    /// Moves to the cell before or after `cell` (wrapping to the previous or
    /// next row), selecting its text when `select`. `false` at the table's ends.
    fn move_to_neighbour(&mut self, cell: &Cell, forward: bool, select: bool) -> bool {
        let target = {
            let doc = self.document_cache.borrow();
            let table = &doc.tables()[cell.table];
            let text = self.buffer.text();
            let content = &text[cell.line_start..doc.line_end_byte(cell.line)];
            let count = cell_ranges(content).len();
            match (forward, cell.column) {
                (true, c) if c + 1 < count => Some((cell.line, c + 1)),
                (true, _) => row_after(table, cell.line).map(|line| (line, 0)),
                (false, c) if c > 0 => Some((cell.line, c - 1)),
                (false, _) => row_before(table, cell.line).map(|line| (line, usize::MAX)),
            }
        };
        let Some((line, column)) = target else {
            return false;
        };
        let Some(next) = self.cell_at_line(line, column) else {
            return false;
        };
        if select {
            self.buffer.set_cursor(next.lo, false);
            self.buffer.set_cursor(next.hi, true);
        } else {
            self.buffer
                .set_cursor(if forward { next.lo } else { next.hi }, false);
        }
        true
    }

    /// Tab and Shift+Tab. Tab in the last cell adds a row. `false` outside a table.
    pub(super) fn tab_in_table(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cell) = self.editing_cell() else {
            return false;
        };
        if !self.move_to_neighbour(&cell, !backwards, true) && !backwards {
            self.append_row(&cell, 0, window, cx);
        }
        cx.notify();
        true
    }

    /// Up and Down: through the cell's wrapped lines, then to the same column
    /// of the neighbouring row. At the table's edge the caret goes to the end
    /// of the cell's text (selecting it with `shift`).
    fn move_vertically(
        &mut self,
        cell: &Cell,
        down: bool,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.vertical_target(cell, down, window, cx);
        match (target, shift) {
            (Some(at), false) => self.buffer.set_cursor(at, false),
            (Some(at), true) => {
                // A selection stays inside the cell it started in.
                let end = if (cell.lo..=cell.hi).contains(&at) {
                    at
                } else if down {
                    cell.hi
                } else {
                    cell.lo
                };
                self.buffer.set_cursor(end, true);
            }
            (None, shift) => self
                .buffer
                .set_cursor(if down { cell.hi } else { cell.lo }, shift),
        }
    }

    /// Where Up or Down from the caret in `cell` lands: the same x on the
    /// cell's next wrapped line, else on the neighbouring row's cell in the same
    /// column. `None` at the table's edge.
    fn vertical_target(
        &self,
        cell: &Cell,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let params = render_params(self, window, cx);
        {
            let doc = self.document_cache.borrow();
            let table = &doc.tables()[cell.table];
            let columns = TableColumns::of(
                table,
                params.available_width,
                (&params.font, params.font_size, params.text_color),
                window,
            );
            let text = self.buffer.text();
            let content = &text[cell.line_start..doc.line_end_byte(cell.line)];
            let at = self.buffer.cursor() - cell.line_start;
            let (row, x) = table::caret_at(content, &columns.widths, cell.column, at);
            let rows = table::layout_row_editing(content, &columns.widths, Some(cell.column))
                [cell.column]
                .lines
                .len();
            let in_cell = if down { row + 1 < rows } else { row > 0 };
            if in_cell {
                let wanted = if down { row + 1 } else { row - 1 };
                let lines = &table::layout_row_editing(content, &columns.widths, Some(cell.column))
                    [cell.column]
                    .lines;
                let offset =
                    table::offset_in_line(content, &columns.widths, cell.column, &lines[wanted], x);
                Some(cell.line_start + offset)
            } else {
                let neighbour = if down {
                    row_after(table, cell.line)
                } else {
                    row_before(table, cell.line)
                };
                match neighbour {
                    None => None,
                    Some(line) => {
                        let start = doc.line_start_byte(line);
                        let other = &text[start..doc.line_end_byte(line)];
                        let column = cell.column.min(cell_ranges(other).len().saturating_sub(1));
                        let lines =
                            &table::layout_row_editing(other, &columns.widths, Some(column))
                                [column]
                                .lines;
                        let range = if down {
                            &lines[0]
                        } else {
                            &lines[lines.len() - 1]
                        };
                        let offset =
                            table::offset_in_line(other, &columns.widths, column, range, x);
                        Some(start + offset)
                    }
                }
            }
        }
    }

    /// Enter: the same column of the next row. In the last row it adds one; in
    /// an empty last row it removes it and leaves the table instead.
    fn enter_in_cell(&mut self, cell: &Cell, window: &mut Window, cx: &mut Context<Self>) {
        let next = {
            let doc = self.document_cache.borrow();
            row_after(&doc.tables()[cell.table], cell.line)
        };
        if let Some(line) = next {
            if let Some(target) = self.cell_at_line(line, cell.column) {
                self.buffer.set_cursor(target.lo, false);
            }
            return;
        }
        let empty = {
            let doc = self.document_cache.borrow();
            let content = &self.buffer.text()[cell.line_start..doc.line_end_byte(cell.line)];
            cell_ranges(content)
                .into_iter()
                .all(|raw| content[edit_range(content, raw)].is_empty())
                && cell.line > doc.tables()[cell.table].start_line + 1
        };
        if empty {
            self.remove_last_row(cell, window, cx);
        } else {
            self.append_row(cell, cell.column, window, cx);
        }
    }

    /// Adds an empty row after the table's last and puts the caret in its `column`.
    fn append_row(
        &mut self,
        cell: &Cell,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (at, row) = {
            let doc = self.document_cache.borrow();
            let table = &doc.tables()[cell.table];
            let end = doc.line_end_byte(table.end_line - 1);
            (end, format!("\n{}", empty_row(table.columns.len())))
        };
        self.replace_in_buffer(at..at, &row, window, cx);
        // After `\n|`, each cell is one space wide: its edit position is the
        // start of that space.
        self.buffer.set_cursor(at + 2 + 2 * column, false);
    }

    /// Deletes the empty last row and puts the caret on the line after the
    /// table, which stays blank so the table ends there.
    fn remove_last_row(&mut self, cell: &Cell, window: &mut Window, cx: &mut Context<Self>) {
        let range = {
            let doc = self.document_cache.borrow();
            let text = self.buffer.text();
            let end = doc.line_end_byte(cell.line);
            let next_blank = doc.line_count() > cell.line + 1
                && text[end + 1..doc.line_end_byte(cell.line + 1)]
                    .trim()
                    .is_empty();
            // A blank line after the table already ends it; otherwise the
            // emptied row itself becomes that line.
            let whole = next_blank && end < text.len();
            cell.line_start..if whole { end + 1 } else { end }
        };
        self.replace_in_buffer(range, "", window, cx);
        self.buffer.set_cursor(cell.line_start, false);
    }

    /// Esc: the caret to the line after the table, adding one at the end of the note.
    fn leave_table(&mut self, cell: &Cell, window: &mut Window, cx: &mut Context<Self>) {
        let (after, at_end) = {
            let doc = self.document_cache.borrow();
            let end_line = doc.tables()[cell.table].end_line;
            (
                (end_line < doc.line_count()).then(|| doc.line_start_byte(end_line)),
                doc.line_end_byte(end_line - 1),
            )
        };
        match after {
            Some(start) => self.buffer.set_cursor(start, false),
            None => {
                self.replace_in_buffer(at_end..at_end, "\n", window, cx);
                self.buffer.set_cursor(at_end + 1, false);
            }
        }
    }

    /// The table cell the caret is in, from the caret itself (so it also holds
    /// in raw mode and right after a click, before the next render's sync):
    /// `(table, row within the table, column)`.
    fn table_cell_at_caret(&self) -> Option<(usize, usize, usize)> {
        let doc = self.document_cache.borrow();
        let cursor = self.buffer.cursor();
        let line = doc.line_at(cursor);
        let table = table_index_at(doc.tables(), line)?;
        let start = doc.line_start_byte(line);
        let content = &self.buffer.text()[start..doc.line_end_byte(line)];
        let row = line - doc.tables()[table].start_line;
        Some((table, row, cell_at(content, cursor - start)))
    }

    /// What the table menu needs to know about the caret's cell, or `None`
    /// outside a table (then there is no menu).
    pub(in crate::text_editor) fn table_menu_state(&self) -> Option<TableMenuState> {
        let (table, row, column) = self.table_cell_at_caret()?;
        let doc = self.document_cache.borrow();
        let columns = &doc.tables()[table].columns;
        let column = column.min(columns.len().checked_sub(1)?);
        let text = self.buffer.text();
        let lines: Vec<&str> = (0..doc.tables()[table].end_line - doc.tables()[table].start_line)
            .map(|i| {
                let line = doc.tables()[table].start_line + i;
                &text[doc.line_start_byte(line)..doc.line_end_byte(line)]
            })
            .collect();
        Some(TableMenuState {
            align: columns[column].align,
            can_edit_row: table::ops::enabled(&lines, row, Op::DeleteRow),
            can_delete_column: table::ops::enabled(&lines, row, Op::DeleteColumn),
        })
    }

    /// Whether byte `offset` is on a line of a table.
    pub(in crate::text_editor) fn is_table_line_at(&self, offset: usize) -> bool {
        let doc = self.document_cache.borrow();
        table_index_at(doc.tables(), doc.line_at(offset)).is_some()
    }

    /// A table shortcut: runs `op` in a table (even when it does not apply
    /// there), and outside one gives the key back to the editor.
    pub(in crate::text_editor) fn table_action(
        &mut self,
        op: Op,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.table_cell_at_caret().is_some() {
            self.run_table_op(op, window, cx);
        } else {
            cx.propagate();
        }
    }

    /// Runs `op` on the table around the caret as one undoable edit. `false`
    /// outside a table or when `op` does not apply there.
    pub(in crate::text_editor) fn run_table_op(
        &mut self,
        op: Op,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((table, row, column)) = self.table_cell_at_caret() else {
            return false;
        };
        self.edit_table(table, window, cx, |old| {
            table::ops::apply(old, row, column, op)
        })
    }

    /// A click on a `+` control: adds the row or column at its boundary.
    pub(in crate::text_editor) fn add_at_handle(
        &mut self,
        table: usize,
        handle: Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let columns = self.document_cache.borrow().tables()[table].columns.len();
        self.edit_table(table, window, cx, |old| match handle {
            Handle::AddColumn(b) if b < columns => table::ops::apply(old, 0, b, Op::ColumnLeft),
            Handle::AddColumn(_) => table::ops::apply(old, 0, columns - 1, Op::ColumnRight),
            // The boundary is the top of row `b`: add below row `b - 1`, whose
            // line is the header's, or one past the delimiter for a body row.
            Handle::AddRow(b) => {
                let row = if b == 1 { 0 } else { b };
                table::ops::apply(old, row, 0, Op::RowBelow)
            }
            _ => None,
        });
    }

    /// The end of a grip drag: moves the column or row to the drop boundary.
    pub(in crate::text_editor) fn finish_table_drag(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(TableDrag { table, grip, over }) = self.table_drag.take() else {
            return;
        };
        self.edit_table(table, window, cx, |old| match grip {
            Handle::ColumnGrip(from) => table::ops::move_column(old, from, over, 0),
            // Row `r` is line `r + 1`; boundary `over` is the top of row `over`.
            Handle::RowGrip(from) => table::ops::move_row(old, from + 1, over + 1, 0),
            _ => None,
        });
        cx.notify();
    }

    /// Applies what `make` does to table `table`'s lines as one undoable edit,
    /// replacing only the lines that differ, and puts the caret where it says.
    fn edit_table(
        &mut self,
        table: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        make: impl FnOnce(&[&str]) -> Option<table::ops::Edited>,
    ) -> bool {
        let (start_line, change) = {
            let doc = self.document_cache.borrow();
            let t = &doc.tables()[table];
            let text = self.buffer.text();
            let old: Vec<&str> = (t.start_line..t.end_line)
                .map(|l| &text[doc.line_start_byte(l)..doc.line_end_byte(l)])
                .collect();
            let Some(edited) = make(&old) else {
                return false;
            };
            let at = |line: usize| doc.line_start_byte(t.start_line + line);
            let end = |line: usize| doc.line_end_byte(t.start_line + line);
            let new: Vec<&str> = edited.lines.iter().map(String::as_str).collect();
            let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let tail = old[head..]
                .iter()
                .rev()
                .zip(new[head..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let (old_end, new_end) = (old.len() - tail, new.len() - tail);
            let joined = new[head..new_end].join("\n");
            let (range, text) = if head == old_end {
                // Lines only added.
                if head < old.len() {
                    (at(head)..at(head), format!("{joined}\n"))
                } else {
                    (end(head - 1)..end(head - 1), format!("\n{joined}"))
                }
            } else if head == new_end {
                // Lines only removed.
                if old_end < old.len() {
                    (at(head)..at(old_end), String::new())
                } else {
                    (end(head - 1)..end(old_end - 1), String::new())
                }
            } else {
                (at(head)..end(old_end - 1), joined)
            };
            (t.start_line, (range, text, edited.caret))
        };
        let (range, text, (caret_row, caret_column)) = change;
        // A move first, so this edit never merges into the typing before it.
        self.buffer.set_cursor(range.start, false);
        self.replace_in_buffer(range, &text, window, cx);
        if let Some(cell) = self.cell_at_line(start_line + caret_row, caret_column) {
            self.buffer.set_cursor(cell.lo, false);
        }
        // Indices and positions the controls were laid out for are stale.
        self.table_hover = None;
        cx.notify();
        true
    }
}

/// What the table context menu shows about the caret's cell.
pub(in crate::text_editor) struct TableMenuState {
    pub align: Align,
    /// Not the header or delimiter row: a row can be added above or deleted.
    pub can_edit_row: bool,
    /// The table has more than one column.
    pub can_delete_column: bool,
}

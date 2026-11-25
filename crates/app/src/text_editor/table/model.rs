//! Table detection and row-parsing — the other *block*-level construct
//! besides fenced code (`preview/fence.rs`), needing cross-line state the
//! same way: a line's role (header/delimiter/body row, or not a table at
//! all) and a table's column count/width/alignment all depend on every row
//! of that table together, not on any single line alone.
//!
//! Deliberately independent of `preview/fence.rs`'s fence tracking and
//! `preview/`'s `classify_block_context` — neither knows about the
//! other, same as those two don't know about each other either.
//! `element.rs` only ever consults this module's output for a line already
//! classified `preview::LineRole::Plain`, so a table-like pipe
//! sequence inside a fenced code block is never mistaken for a real table
//! (this module's own scan would still "detect" it, since it has no fence
//! awareness, but nothing ever looks that detection up for a fenced line).
//! A table nested inside a blockquote or list item (`> | a | b |`) isn't
//! recognized either — out of scope, same spirit as every other
//! compounding limit in this editor.
//!
//! Rendering a table as an actually-aligned grid (not just markup hidden in
//! place, like every other construct) is `preview/`'s
//! `build_table_row`/`build_table_rule` — made tractable by this editor's
//! font always being monospace (see `element.rs`'s `render_editor`): a
//! column's width is a plain character count, with no pixel measurement
//! needed to line columns up across rows.

use std::ops::Range;

use super::super::math::{Macros, NO_MACROS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One table column's rendering width (in characters) and alignment, both
/// resolved from the table's delimiter row — width is the *widest* cell
/// across the header and every body row, not just the delimiter row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub width: usize,
    /// The longest word of any cell, in characters (a `<br>` ends a word): the
    /// least width at which no word is broken (#46).
    pub word: usize,
    pub align: Align,
}

/// One detected table: its header row's line index, the line index right
/// after its last body row, the byte range it spans in the whole document,
/// and its resolved columns. Its byte range (from `DocumentCache::table_byte_range`) is what `element.rs` checks the
/// cursor against to decide whether the *whole* table should render raw —
/// unlike every other construct's single "active line," a table's
/// row/column alignment can't be resolved from one line in isolation, so
/// there's no sensible way to keep the rest of the grid rendered while one
/// row inside it goes raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub start_line: usize,
    pub end_line: usize,
    pub columns: Vec<Column>,
}

impl Table {
    pub fn is_delimiter(&self, line: usize) -> bool {
        line == self.start_line + 1
    }

    /// Whether `line` is this table's header row — the one line that gets a
    /// top border frame (`┌─┬─┐`) drawn above it, so the grid reads as a
    /// fully closed box rather than open along its top edge.
    pub fn is_first_row(&self, line: usize) -> bool {
        line == self.start_line
    }
}

/// One row's parsed cells.
pub struct Row {
    pub cells: Vec<Cell>,
}

/// One cell's trimmed, unescaped (`\|` → a literal `|`) text.
pub struct Cell {
    pub text: String,
}

/// Splits a `|`-delimited row into [`Cell`]s. A backslash-escaped pipe
/// (`\|`) doesn't split a cell — it's unescaped to a literal `|` in that
/// cell's `text` — and a leading/trailing `|` is optional and, if present,
/// isn't part of any cell's own range.
pub fn scan_row(line: &str) -> Row {
    let mut cells = Vec::new();
    for_each_cell_range(line, |raw| cells.push(make_cell(line, raw)));
    Row { cells }
}

/// Calls `visit` with the byte range of each cell of `line`, in order, and
/// returns `(has_leading_pipe, has_trailing_pipe)`. The one place the
/// cell-splitting rules live, shared by [`scan_row`] and [`row_line_count`].
pub(in crate::text_editor) fn for_each_cell_range(
    line: &str,
    mut visit: impl FnMut(Range<usize>),
) -> (bool, bool) {
    let bytes = line.as_bytes();
    let has_leading_pipe = bytes.first() == Some(&b'|');
    let start = usize::from(has_leading_pipe);

    let mut cell_start = start;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'|') {
            i += 2;
            continue;
        }
        if bytes[i] == b'|' {
            visit(cell_start..i);
            cell_start = i + 1;
        }
        i += 1;
    }
    // If the last byte consumed was a pipe, `cell_start` landed exactly at
    // the end with nothing left to flush — that's a real trailing pipe,
    // not a final cell of its own.
    let has_trailing_pipe = cell_start == bytes.len() && cell_start > start;
    if !has_trailing_pipe {
        visit(cell_start..bytes.len());
    }
    (has_leading_pipe, has_trailing_pipe)
}

/// Written in a cell where a line must end: the only HTML a cell understands,
/// since markdown has no line break inside a table row.
pub const LINE_BREAK: &str = "<br>";

/// How many lines `line` renders on when each of its cells is wrapped to
/// `widths` — exactly the number of lines `preview::build_table_row`
/// returns for it, without building them. A cell no longer than its column is
/// wide always fits one line (wrapping only ever removes whitespace), which is
/// nearly every cell, so only an over-wide one is counted word by word. A row
/// with fewer cells than `widths` counts the missing ones as one empty line;
/// extra cells are ignored.
pub fn row_line_count(line: &str, widths: &[usize]) -> usize {
    row_line_count_in(line, widths, None, &NO_MACROS)
}

/// [`row_line_count`] for a row whose cell `editing` is being edited: that cell
/// shows its text verbatim (see `table::geometry::wrap_raw`), so it wraps
/// differently. The other cells draw their formulas, expanded with `macros`.
pub fn row_line_count_in(
    line: &str,
    widths: &[usize],
    editing: Option<usize>,
    macros: &Macros,
) -> usize {
    let mut lines = 1;
    let mut column = 0;
    for_each_cell_range(line, |raw| {
        if let Some(&width) = widths.get(column) {
            if editing == Some(column) {
                let shown = super::geometry::edit_range(line, raw);
                lines = lines.max(super::geometry::raw_line_count(&line[shown], width));
            } else {
                let cell = line[raw].trim();
                // Bytes are never fewer than characters.
                if cell.len() > width || cell.contains(LINE_BREAK) || cell.contains('$') {
                    lines = lines.max(wrapped_line_count_in(cell, width, macros));
                }
            }
        }
        column += 1;
    });
    lines
}

/// How many rows each line of the row takes when it is shaped as `entries`
/// rows: more than one when a drawn formula in a cell is taller than a row.
pub fn row_scale(
    line: &str,
    widths: &[usize],
    (editing, macros): (Option<usize>, &Macros),
    entries: usize,
) -> usize {
    (entries / row_line_count_in(line, widths, editing, macros)).max(1)
}

/// A run of a cell that the wrapping places whole.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::text_editor) struct Chunk {
    /// Where it is in the cell text.
    pub range: Range<usize>,
    /// Characters it takes on a line (`\|` shows as one, a drawn formula as
    /// many as it reserves).
    pub len: usize,
    /// It follows the chunk before it with no space between.
    pub glued: bool,
}

/// Characters `word` takes once `\|` shows as `|`.
pub(in crate::text_editor) fn shown_len(word: &str) -> usize {
    word.chars().count() - word.matches("\\|").count()
}

/// `text` as the runs the greedy fill places. Spaces separate them, except
/// inside a drawn formula (`formulas`: raw range and reserved characters, in
/// order): it is one run of its drawn width, so it is never cut. A word that
/// touches a formula is cut at the formula's edges into glued runs. A formula
/// wider than `width` is not kept whole (it shows as the text it is, which
/// wraps as text).
pub(in crate::text_editor) fn chunks(
    text: &str,
    formulas: &[(Range<usize>, usize)],
    width: usize,
) -> Vec<Chunk> {
    let kept: Vec<_> = formulas
        .iter()
        .filter(|(_, w)| *w <= width.max(1))
        .collect();
    let (mut out, mut next, mut at) = (Vec::new(), 0, 0);
    let mut prev_end = None;
    while at < text.len() {
        if let Some((range, len)) = kept.get(next).filter(|(r, _)| r.start == at) {
            out.push(Chunk {
                range: range.clone(),
                len: *len,
                glued: prev_end == Some(at),
            });
            (at, next, prev_end) = (range.end, next + 1, Some(range.end));
            continue;
        }
        let Some(c) = text[at..].chars().next() else {
            break;
        };
        if c.is_whitespace() {
            at += c.len_utf8();
            continue;
        }
        let start = at;
        let stop = kept.get(next).map_or(text.len(), |(r, _)| r.start);
        while at < stop
            && text[at..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
        out.push(Chunk {
            range: start..at,
            len: shown_len(&text[start..at]),
            glued: prev_end == Some(start),
        });
        prev_end = Some(at);
    }
    out
}

/// [`wrapped_line_count`] with the cell's formulas drawn (see [`chunks`]).
pub fn wrapped_line_count_in(cell: &str, width: usize, macros: &Macros) -> usize {
    if cell.contains(LINE_BREAK) {
        return cell
            .split(LINE_BREAK)
            .map(|part| wrapped_line_count_in(part, width, macros))
            .sum();
    }
    if !cell.contains('$') {
        return wrapped_line_count(cell, width);
    }
    let formulas = super::super::preview::cell_formulas(cell, macros);
    if formulas.is_empty() {
        return wrapped_line_count(cell, width);
    }
    let width = width.max(1);
    let (mut lines, mut used) = (0, 0);
    for chunk in chunks(cell, &formulas, width) {
        let len = chunk.len;
        let separator = usize::from(used > 0 && !chunk.glued);
        if used + separator + len <= width {
            used += separator + len;
        } else if len <= width {
            lines += 1;
            used = len;
        } else {
            lines += usize::from(used > 0) + (len - 1) / width;
            used = len - (len - 1) / width * width;
        }
    }
    lines + usize::from(used > 0 || lines == 0)
}

/// `wrap_cell_text(&cell.replace("\\|", "|"), width).len()` without building a
/// string: the same greedy fill, counting instead of copying. `cell` is a raw
/// (still escaped) cell; `\|` counts as the one character it renders as.
pub fn wrapped_line_count(cell: &str, width: usize) -> usize {
    if cell.contains(LINE_BREAK) {
        return cell
            .split(LINE_BREAK)
            .map(|part| wrapped_line_count(part, width))
            .sum();
    }
    let width = width.max(1);
    let (mut lines, mut used) = (0, 0);
    for word in cell.split_whitespace() {
        let len = word.chars().count() - word.matches("\\|").count();
        let separator = usize::from(used > 0);
        if used + separator + len <= width {
            used += separator + len;
        } else if len <= width {
            // Starts a new line.
            lines += 1;
            used = len;
        } else {
            // Longer than a whole line: hard-broken into full lines, the rest
            // left open for the next word.
            lines += usize::from(used > 0) + (len - 1) / width;
            used = len - (len - 1) / width * width;
        }
    }
    lines + usize::from(used > 0 || lines == 0)
}

/// How many cells [`scan_row`] would return, without building them.
fn cell_count(line: &str) -> usize {
    let mut count = 0;
    for_each_cell_range(line, |_| count += 1);
    count
}

/// `make_cell(line, raw).text.chars().count()` without building the text.
fn cell_width(line: &str, raw: Range<usize>) -> usize {
    let cell = &line[raw];
    let bytes = cell.as_bytes();
    // Plain ASCII (no escapes, and no vertical tab, which `trim_ascii` keeps
    // but `trim` strips): every char is one byte.
    if bytes.iter().all(|&b| b < 0x80 && b != b'\\' && b != 0x0b) {
        bytes.trim_ascii().len()
    } else if bytes.contains(&b'\\') {
        cell.replace("\\|", "|").trim().chars().count()
    } else {
        cell.trim().chars().count()
    }
}

/// The longest word of `text`, in characters: a run without whitespace, where a
/// `<br>` also ends a word and an escaped pipe (`\|`) is one character. One
/// pass over the bytes, no allocation: this runs for every cell of a table.
fn longest_word(text: &str) -> usize {
    let bytes = text.as_bytes();
    let (mut best, mut run, mut i) = (0, 0, 0);
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'<' && bytes[i..].starts_with(b"<br>") {
            best = best.max(run);
            run = 0;
            i += 4;
        } else if byte == b'\\' && bytes.get(i + 1) == Some(&b'|') {
            run += 1;
            i += 2;
        } else if byte < 0x80 {
            if (byte as char).is_whitespace() {
                best = best.max(run);
                run = 0;
            } else {
                run += 1;
            }
            i += 1;
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                best = best.max(run);
                run = 0;
            } else {
                run += 1;
            }
            i += c.len_utf8();
        }
    }
    best.max(run)
}

/// A cell's natural width and longest word in characters, with each formula it
/// draws counted at its drawn width (#47: a short formula with long source takes
/// the room it needs, a long one no more). Only a cell with a `$` is looked at
/// for formulas. The classifier knows no macros, so a formula that needs one is
/// counted as its source, which is also how it is shown without them.
fn text_metrics(text: &str) -> (usize, usize) {
    let plain = || (text.chars().count(), longest_word(text));
    if !text.contains('$') {
        return plain();
    }
    let text = text.trim();
    let (mut width, mut word, mut drawn) = (0, 0, false);
    for part in text.split(LINE_BREAK) {
        let formulas = super::super::preview::cell_formulas(part, &NO_MACROS);
        drawn |= !formulas.is_empty();
        let (mut line, mut run) = (0, 0);
        for (k, chunk) in chunks(part, &formulas, usize::MAX).iter().enumerate() {
            if !chunk.glued {
                word = word.max(run);
                run = 0;
                line += usize::from(k > 0);
            }
            run += chunk.len;
            line += chunk.len;
        }
        word = word.max(run);
        width = width.max(line);
    }
    if drawn { (width, word) } else { plain() }
}

/// [`text_metrics`] of the cell at `raw` in `line`, cheap for one without a `$`.
fn cell_metrics(line: &str, raw: Range<usize>) -> (usize, usize) {
    if line[raw.clone()].contains('$') {
        text_metrics(line[raw].replace("\\|", "|").trim())
    } else {
        (cell_width(line, raw.clone()), cell_word(line, raw))
    }
}

/// [`longest_word`] of the cell at `raw` in `line`.
fn cell_word(line: &str, raw: Range<usize>) -> usize {
    longest_word(&line[raw])
}

fn make_cell(line: &str, raw: Range<usize>) -> Cell {
    Cell {
        text: line[raw].replace("\\|", "|").trim().to_string(),
    }
}

/// The alignment a delimiter cell (e.g. `---`, `:--`, `--:`, `:-:`)
/// specifies, or `None` if `cell` isn't a valid delimiter cell at all (not
/// made up of a run of `-` optionally bounded by `:`).
fn delimiter_align(cell: &str) -> Option<Align> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    let start = usize::from(left);
    let end = cell.len() - usize::from(right);
    // A single ":" sets both `left` and `right` from the same byte — guard
    // against that overlap rather than slicing a backwards range.
    if start > end {
        return None;
    }
    let dashes = &cell[start..end];
    if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    })
}

/// `line`'s per-column alignments if it's a valid delimiter row for a
/// table with `expected_columns` columns — GFM requires its cell count to
/// match the header's *exactly* (unlike body rows, which may have more or
/// fewer cells than the header, padded/truncated when rendered).
fn parse_delimiter_row(line: &str, expected_columns: usize) -> Option<Vec<Align>> {
    if cell_count(line) != expected_columns {
        return None;
    }
    // A cell with an escaped pipe is never made of dashes, so the raw text
    // gives the same answer as the unescaped one.
    let mut aligns = Vec::new();
    let mut valid = true;
    for_each_cell_range(line, |raw| {
        if !valid {
            return;
        }
        match delimiter_align(&line[raw]) {
            Some(align) => aligns.push(align),
            None => valid = false,
        }
    });
    valid.then_some(aligns)
}

/// The index of the table containing `line`, if any. Tables are sorted and
/// disjoint, so this is a binary search; it replaces a per-line lookup array
/// that was 16 bytes per document line.
pub(in crate::text_editor) fn table_index_at(tables: &[Table], line: usize) -> Option<usize> {
    let i = tables.partition_point(|t| t.end_line <= line);
    tables.get(i).filter(|t| t.start_line <= line).map(|_| i)
}

/// A line is a plausible table row if it contains at least one `|` — the
/// cheapest possible check, same spirit as this module's other
/// simplifications; the real validation happens via the *next* line having
/// to be a valid delimiter row for a table to be recognized at all.
pub(in crate::text_editor) fn is_pipe_row(line: &str) -> bool {
    // The byte search skips `str::contains`'s char-pattern machinery, which
    // costs more than the scan itself on a short line.
    line.as_bytes().contains(&b'|')
}

/// Every table in `source`, in order: the same result as
/// [`classify_tables`]' second value, computed in one streaming pass without
/// materializing the document's lines or a per-line lookup (which cost
/// ~12MB of transient allocation on the 320K-line stress fixture).
pub(in crate::text_editor) fn classify_table_list(source: &str) -> Vec<Table> {
    classify_table_list_from(source.split('\n'))
}

/// [`classify_table_list`] over lines the caller already split.
pub(in crate::text_editor) fn classify_table_list_from<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Vec<Table> {
    let mut lines = lines.enumerate().peekable();
    let mut tables = Vec::new();

    while let Some((start_line, header)) = lines.next() {
        if !is_pipe_row(header) {
            continue;
        }
        let Some(&(_, delimiter)) = lines.peek() else {
            break;
        };
        // The delimiter row decides, so check it before scanning the header
        // (nearly every pipe row is not a header).
        let Some(aligns) = parse_delimiter_row(delimiter, cell_count(header)) else {
            continue;
        };
        let header_row = scan_row(header);
        lines.next();
        let mut end_line = start_line + 2;

        let mut columns: Vec<Column> = header_row
            .cells
            .iter()
            .zip(aligns)
            .map(|(cell, align)| {
                let (width, word) = text_metrics(&cell.text);
                Column {
                    width: width.max(3),
                    word,
                    align,
                }
            })
            .collect();
        while let Some(&(i, row)) = lines.peek() {
            if !is_pipe_row(row) {
                break;
            }
            let mut index = 0;
            for_each_cell_range(row, |raw| {
                if let Some(column) = columns.get_mut(index) {
                    let (width, word) = cell_metrics(row, raw);
                    column.word = column.word.max(word);
                    column.width = column.width.max(width);
                }
                index += 1;
            });
            end_line = i + 1;
            lines.next();
        }

        tables.push(Table {
            start_line,
            end_line,
            columns,
        });
    }
    tables
}

/// Resolves every table in the whole `source` document: for each line, the
/// index into the returned `Vec<Table>` it belongs to (if any), and each
/// table's resolved columns. A single sequential pass, mirroring
/// `preview::fence::classify_lines`'s fence-tracking and
/// `preview::classify_block_context`'s list-nesting pass — the same
/// kind of whole-document state this module's own doc comment explains
/// tables need and single-line detection can't provide.
#[cfg(test)]
pub fn classify_tables(source: &str) -> (Vec<Option<usize>>, Vec<Table>) {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut table_for_line = vec![None; lines.len()];
    let mut tables = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        if i + 1 >= lines.len() || !is_pipe_row(lines[i]) {
            i += 1;
            continue;
        }
        let header_row = scan_row(lines[i]);
        let Some(aligns) = parse_delimiter_row(lines[i + 1], header_row.cells.len()) else {
            i += 1;
            continue;
        };

        let start = i;
        let mut end = i + 2;
        while end < lines.len() && is_pipe_row(lines[end]) {
            end += 1;
        }

        let mut columns: Vec<Column> = header_row
            .cells
            .iter()
            .zip(aligns)
            .map(|(cell, align)| {
                let (width, word) = text_metrics(&cell.text);
                Column {
                    width: width.max(3),
                    word,
                    align,
                }
            })
            .collect();
        for &line in &lines[start + 2..end] {
            for (column, cell) in columns.iter_mut().zip(scan_row(line).cells.iter()) {
                let (width, word) = text_metrics(&cell.text);
                column.word = column.word.max(word);
                column.width = column.width.max(width);
            }
        }

        for entry in &mut table_for_line[start..end] {
            *entry = Some(tables.len());
        }
        tables.push(Table {
            start_line: start,
            end_line: end,
            columns,
        });
        i = end;
    }

    (table_for_line, tables)
}

/// The minimum a column is ever shrunk to by [`cap_column_widths`] — small
/// enough to still show a few characters per wrapped line, large enough
/// that a many-column table doesn't collapse every column to nothing.
pub const MIN_COLUMN_WIDTH: usize = 3;

/// The characters `columns` columns spend on gaps: `3 * columns + 1`, plus the
/// space `table.cell_padding` adds on both sides of every cell.
fn border_overhead(columns: usize) -> usize {
    let extra = 2.0 * crate::settings::cell_padding() / super::super::math::MONO_ADVANCE_EM;
    3 * columns + 1 + (extra * columns as f32).ceil() as usize
}

/// Whether `columns` keep their natural widths within `max_total_chars`
/// (or there is no cap). Then every cell fits its column, so no row wraps.
pub fn fits_unshrunk(columns: &[Column], max_total_chars: impl Into<Option<usize>>) -> bool {
    let Some(max) = max_total_chars.into() else {
        return true;
    };
    let content_budget = max.saturating_sub(border_overhead(columns.len()));
    columns.iter().map(|c| c.width).sum::<usize>() <= content_budget
}

/// Shrinks `columns`' natural widths — the widest ones to an equal share, the
/// narrow ones untouched, floored at [`MIN_COLUMN_WIDTH`] unless even that cannot fit — so their total
/// (plus border/padding overhead) fits within `max_total_chars`, if it
/// doesn't already. A no-op (returns the natural widths unchanged) when it
/// already fits. Border overhead is `3 * columns.len() + 1` characters: a
/// leading 2-character gap and a trailing 2-character gap (both blank —
/// the table's actual left/right border lines are painted as real vector
/// quads over them, see `column_divider_char_offsets` below, not drawn as
/// text), plus a 3-character blank gap between every pair of adjacent
/// columns — see `preview::build_table_row`.
pub fn cap_column_widths(columns: &[Column], max_total_chars: usize) -> Vec<usize> {
    let natural: Vec<usize> = columns.iter().map(|c| c.width).collect();
    if columns.is_empty() || fits_unshrunk(columns, max_total_chars) {
        return natural;
    }
    let content_budget = max_total_chars.saturating_sub(border_overhead(columns.len()));

    let floor_total = MIN_COLUMN_WIDTH * columns.len();
    if content_budget <= floor_total {
        // Too narrow even for the floor: share what there is (a table in a
        // narrow float would otherwise run out of its column), a character
        // each at the least.
        let each = (content_budget / columns.len()).max(1);
        let extra = content_budget.saturating_sub(each * columns.len());
        return (0..columns.len())
            .map(|i| each + usize::from(i < extra))
            .collect();
    }

    // Whole words first (#46): no column goes below its longest word (nor
    // `MIN_COLUMN_WIDTH`, nor its natural width) while the words of all
    // columns fit together; only when they do not is a word broken.
    let floors: Vec<usize> = columns
        .iter()
        .map(|c| c.word.max(MIN_COLUMN_WIDTH).min(c.width))
        .collect();
    if floors.iter().sum::<usize>() <= content_budget {
        return fill_above(&natural, &floors, content_budget, false);
    }
    // Not all words fit: the longest words are shortened to a common length,
    // so breaking is spread over the columns (a column's word count is not
    // known here, and one column holding many long words would suffer most).
    let minimum = vec![MIN_COLUMN_WIDTH; floors.len()];
    fill_above(&floors, &minimum, content_budget, true)
}

/// Widths between `floors` and `natural` using `budget` characters, water-filled:
/// a column narrower than the common level keeps its natural width, the wider
/// ones share the rest equally (the widest take the odd characters), none
/// below its floor. `floors` must sum to at most `budget`.
fn fill_above(
    natural: &[usize],
    floors: &[usize],
    budget: usize,
    nearest_first: bool,
) -> Vec<usize> {
    let at = |level: usize| -> Vec<usize> {
        natural
            .iter()
            .zip(floors)
            .map(|(&n, &f)| level.clamp(f, n.max(f)))
            .collect()
    };
    // The highest level whose widths still fit.
    let (mut low, mut high) = (0, natural.iter().copied().max().unwrap_or(0));
    while low < high {
        let mid = (low + high).div_ceil(2);
        if at(mid).iter().sum::<usize>() <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut widths = at(low);
    let mut spare = budget - widths.iter().sum::<usize>();
    // The odd characters go to the widest columns still below their natural
    // width; with `nearest_first` to those nearest it, where one more may make a
    // word fit.
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by_key(|&i| {
        let gap = if nearest_first {
            natural[i] - widths[i]
        } else {
            0
        };
        (gap, std::cmp::Reverse((natural[i], i)))
    });
    for i in order {
        if spare == 0 {
            break;
        }
        if widths[i] < natural[i] {
            widths[i] += 1;
            spare -= 1;
        }
    }
    widths
}

/// `columns`' widths as actually used for rendering — [`cap_column_widths`]
/// if `max_total_chars` is given, else each column's own natural width
/// verbatim. The one place both `preview::build_table_row` and
/// `element.rs`'s border-painting code (which needs the *same* resolved
/// widths `build_table_row` used, to position its vector lines at exactly
/// the gaps that row's text actually left blank) make this decision, so
/// they can't silently disagree.
pub fn resolve_widths(columns: &[Column], max_total_chars: Option<usize>) -> Vec<usize> {
    match max_total_chars {
        Some(max) => cap_column_widths(columns, max),
        None => columns.iter().map(|c| c.width).collect(),
    }
}

/// Word-wraps `text` into lines no wider than `width` *characters* — a
/// plain greedy fill (pack whole words onto the current line until the
/// next one wouldn't fit, then start a new one), character-counted rather
/// than pixel-measured since this editor's font is always monospace. A
/// single word still too long for an entire empty line on its own is
/// hard-broken at `width` characters rather than left overflowing.
/// `width` is floored at `1` so a pathologically narrow cap can't cause a
/// zero-width (infinite) break loop.
#[cfg(test)]
pub fn wrap_cell_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let mut remaining = word;
        loop {
            let sep = usize::from(!current.is_empty());
            if current.chars().count() + sep + remaining.chars().count() <= width {
                if sep == 1 {
                    current.push(' ');
                }
                current.push_str(remaining);
                break;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                continue;
            }
            // `remaining` alone doesn't fit on a fresh, empty line either —
            // hard-break it at `width` characters and keep going with what's
            // left.
            let split_at = remaining
                .char_indices()
                .nth(width)
                .map_or(remaining.len(), |(i, _)| i);
            if split_at == remaining.len() {
                current.push_str(remaining);
                break;
            }
            lines.push(remaining[..split_at].to_string());
            remaining = &remaining[split_at..];
        }
    }

    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_texts(row: &Row) -> Vec<&str> {
        row.cells.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn scan_row_splits_a_simple_row_with_leading_and_trailing_pipes() {
        let row = scan_row("| Name | Age |");
        assert_eq!(cell_texts(&row), vec!["Name", "Age"]);
    }

    #[test]
    fn scan_row_handles_missing_leading_and_trailing_pipes() {
        let row = scan_row("Name | Age");
        assert_eq!(cell_texts(&row), vec!["Name", "Age"]);
    }

    #[test]
    fn scan_row_unescapes_a_backslash_escaped_pipe_without_splitting_the_cell() {
        let row = scan_row(r"| a \| b | second |");
        assert_eq!(cell_texts(&row), vec!["a | b", "second"]);
    }

    #[test]
    fn scan_row_on_a_single_empty_cell() {
        let row = scan_row("| |");
        assert_eq!(cell_texts(&row), vec![""]);
    }

    #[test]
    fn delimiter_align_recognizes_all_four_forms() {
        assert_eq!(delimiter_align("---"), Some(Align::Left));
        assert_eq!(delimiter_align(":--"), Some(Align::Left));
        assert_eq!(delimiter_align("--:"), Some(Align::Right));
        assert_eq!(delimiter_align(":-:"), Some(Align::Center));
    }

    #[test]
    fn delimiter_align_rejects_non_dash_content() {
        assert_eq!(delimiter_align("abc"), None);
        assert_eq!(delimiter_align(""), None);
        assert_eq!(delimiter_align(":"), None);
    }

    #[test]
    fn classify_table_list_matches_classify_tables_on_the_stress_fixture() {
        let Ok(text) = std::fs::read_to_string("benches/fixtures/large_markdown_stress_test.md")
        else {
            return;
        };
        assert_eq!(classify_table_list(&text), classify_tables(&text).1);
    }

    #[test]
    fn classify_tables_recognizes_a_simple_table() {
        let source = "| Name | Age |\n| --- | --- |\n| Alice | 30 |\n| Bob | 25 |";
        let (table_for_line, tables) = classify_tables(source);
        assert_eq!(table_for_line, vec![Some(0), Some(0), Some(0), Some(0)]);
        assert_eq!(tables.len(), 1);
        let table = &tables[0];
        assert_eq!(table.start_line, 0);
        assert_eq!(table.end_line, 4);
        assert_eq!(table.columns.len(), 2);
    }

    #[test]
    fn classify_tables_resolves_alignment_per_column() {
        let source = "| L | C | R |\n| :-- | :-: | --: |\n| a | b | c |";
        let (_, tables) = classify_tables(source);
        let columns = &tables[0].columns;
        assert_eq!(columns[0].align, Align::Left);
        assert_eq!(columns[1].align, Align::Center);
        assert_eq!(columns[2].align, Align::Right);
    }

    #[test]
    fn classify_tables_column_width_is_the_widest_cell_across_every_row() {
        let source = "| Name | Age |\n| --- | --- |\n| Alexandria | 30 |\n| Bo | 9 |";
        let (_, tables) = classify_tables(source);
        // "Alexandria" (10 chars) is wider than the header "Name" (4).
        assert_eq!(tables[0].columns[0].width, 10);
        assert_eq!(tables[0].columns[1].width, 3); // header "Age" is widest here
    }

    #[test]
    fn classify_tables_ends_at_a_blank_line() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n\nnot part of the table";
        let (table_for_line, tables) = classify_tables(source);
        assert_eq!(table_for_line, vec![Some(0), Some(0), Some(0), None, None]);
        assert_eq!(tables[0].end_line, 3);
    }

    #[test]
    fn classify_tables_ends_at_a_non_pipe_line() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\nplain paragraph text";
        let (table_for_line, _) = classify_tables(source);
        assert_eq!(table_for_line, vec![Some(0), Some(0), Some(0), None]);
    }

    #[test]
    fn classify_tables_rejects_a_pipe_row_not_followed_by_a_valid_delimiter() {
        let source = "| A | B |\nnot a delimiter row\n| 1 | 2 |";
        let (table_for_line, tables) = classify_tables(source);
        assert_eq!(table_for_line, vec![None, None, None]);
        assert!(tables.is_empty());
    }

    #[test]
    fn classify_tables_rejects_a_delimiter_row_with_the_wrong_column_count() {
        let source = "| A | B |\n| --- |\n| 1 | 2 |";
        let (table_for_line, tables) = classify_tables(source);
        assert_eq!(table_for_line, vec![None, None, None]);
        assert!(tables.is_empty());
    }

    #[test]
    fn classify_tables_handles_a_body_row_with_fewer_cells_than_the_header() {
        let source = "| A | B | C |\n| --- | --- | --- |\n| 1 |";
        let (_, tables) = classify_tables(source);
        assert_eq!(tables[0].columns.len(), 3);
        // The missing cells shouldn't widen (or otherwise affect) the
        // columns they don't have content for.
        assert_eq!(tables[0].columns[1].width, 3); // "B" padded to the min-3 floor
    }

    #[test]
    fn classify_tables_recognizes_two_separate_tables() {
        let source = "| A |\n| --- |\n| 1 |\n\n| B |\n| --- |\n| 2 |";
        let (table_for_line, tables) = classify_tables(source);
        assert_eq!(
            table_for_line,
            vec![Some(0), Some(0), Some(0), None, Some(1), Some(1), Some(1)]
        );
        assert_eq!(tables.len(), 2);
    }

    /// Padding takes characters of the budget, so a table that just fit no longer does.
    #[test]
    fn cell_padding_costs_budget() {
        let columns = [col(10), col(10)];
        // 7 characters of gaps and 20 of text.
        assert!(fits_unshrunk(&columns, 27));
        let mut t = crate::settings::Typography::default();
        t.table.cell_padding = 0.6;
        crate::settings::set_body(&t);
        let (still, wider) = (fits_unshrunk(&columns, 27), fits_unshrunk(&columns, 31));
        let shrunk = resolve_widths(&columns, Some(27));
        crate::settings::set_body(&crate::settings::Typography::default());
        assert!(!still && wider, "4 more characters of gaps");
        assert!(shrunk.iter().sum::<usize>() <= 16, "{shrunk:?}");
    }

    fn col(width: usize) -> Column {
        Column {
            width,
            word: 1,
            align: Align::Left,
        }
    }

    fn col_with_word(width: usize, word: usize) -> Column {
        Column { word, ..col(width) }
    }

    #[test]
    fn a_column_never_shrinks_below_its_longest_word_while_all_words_fit() {
        // #46: an equal share (here 12 each) would cut the 14-letter word.
        let columns = vec![col_with_word(40, 14), col_with_word(40, 5)];
        let widths = cap_column_widths(&columns, 7 + 24);
        assert_eq!(widths.iter().sum::<usize>(), 24);
        assert!(widths[0] >= 14 && widths[1] >= 5, "{widths:?}");
        assert_eq!(widths, vec![14, 10]);
    }

    #[test]
    fn short_columns_keep_their_natural_width_and_long_words_the_rest() {
        let columns = vec![
            col_with_word(4, 4),
            col_with_word(30, 9),
            col_with_word(30, 9),
        ];
        // Overhead 10; 28 of content: 4 stays, the others share 24.
        assert_eq!(cap_column_widths(&columns, 10 + 28), vec![4, 12, 12]);
    }

    #[test]
    fn when_words_do_not_all_fit_the_longest_are_shortened_to_a_common_length() {
        // Words 5, 10, 9, 5 want 29; 27 fit: the two long ones give a character each.
        let columns = vec![
            col_with_word(5, 5),
            col_with_word(10, 10),
            col_with_word(20, 9),
            col_with_word(5, 5),
        ];
        // The odd character goes to the 9-letter word's column, where it makes it whole.
        assert_eq!(cap_column_widths(&columns, 13 + 27), vec![5, 8, 9, 5]);
    }

    #[test]
    fn words_that_cannot_all_fit_fall_back_to_an_equal_share() {
        let columns = vec![col_with_word(40, 20), col_with_word(40, 20)];
        let widths = cap_column_widths(&columns, 7 + 20);
        assert_eq!(widths, vec![10, 10]);
    }

    #[test]
    fn cap_column_widths_is_a_no_op_when_everything_already_fits() {
        let columns = vec![col(5), col(3)];
        // border overhead for 2 columns = 3*2+1 = 7; 5+3+7 = 15.
        assert_eq!(cap_column_widths(&columns, 20), vec![5, 3]);
    }

    #[test]
    fn cap_column_widths_shrinks_only_the_wide_columns() {
        // Overhead for 3 columns is 10; 40 of content left.
        let columns = vec![col(3), col(4), col(60)];
        assert_eq!(cap_column_widths(&columns, 10 + 40), vec![3, 4, 33]);
        // Two wide columns split what the narrow one leaves, equally.
        let columns = vec![col(5), col(30), col(50)];
        assert_eq!(cap_column_widths(&columns, 10 + 40), vec![5, 17, 18]);
    }

    #[test]
    fn a_growing_cell_takes_space_from_columns_with_room() {
        let fits = |widest: usize| cap_column_widths(&[col(8), col(widest), col(6)], 10 + 40);
        assert_eq!(fits(20), vec![8, 20, 6]);
        assert_eq!(
            fits(30),
            vec![8, 26, 6],
            "grows until the others are as small as they are"
        );
        assert!(fits(40).iter().sum::<usize>() <= 40);
    }

    #[test]
    fn cap_column_widths_keeps_the_minimum_while_it_fits() {
        let columns = vec![col(20), col(20), col(20), col(20)];
        let widths = cap_column_widths(&columns, 3 * 4 + 1 + MIN_COLUMN_WIDTH * 4);
        assert_eq!(widths, vec![MIN_COLUMN_WIDTH; 4]);
    }

    /// Below the minimum only when the box could not fit its room otherwise: a
    /// table in a narrow float stays inside it instead of running out.
    #[test]
    fn cap_column_widths_gives_way_below_the_minimum_to_fit() {
        let columns = vec![col(20), col(20), col(20)];
        // Room for 7 characters of text beside the 10 of borders and padding.
        let widths = cap_column_widths(&columns, 17);
        assert_eq!(widths.iter().sum::<usize>(), 7);
        assert!(widths.iter().all(|&w| w >= 1));
        assert_eq!(
            cap_column_widths(&columns, 5),
            vec![1; 3],
            "a character each at least"
        );
    }

    #[test]
    fn resolve_widths_is_natural_widths_without_a_budget() {
        let columns = vec![col(5), col(3)];
        assert_eq!(resolve_widths(&columns, None), vec![5, 3]);
    }

    #[test]
    fn resolve_widths_caps_when_a_budget_is_given() {
        let columns = vec![col(20), col(20), col(20), col(20)];
        assert_eq!(resolve_widths(&columns, Some(5)), vec![1; 4]);
    }

    #[test]
    fn wrap_cell_text_packs_whole_words_greedily() {
        assert_eq!(
            wrap_cell_text("Alexandria Fitzgerald", 10),
            vec!["Alexandria", "Fitzgerald"]
        );
    }

    #[test]
    fn wrap_cell_text_is_a_single_line_when_it_already_fits() {
        assert_eq!(wrap_cell_text("short", 10), vec!["short"]);
    }

    #[test]
    fn wrap_cell_text_on_empty_text_is_one_empty_line() {
        assert_eq!(wrap_cell_text("", 10), vec![""]);
    }

    #[test]
    fn wrap_cell_text_hard_breaks_a_word_too_long_for_an_empty_line() {
        assert_eq!(
            wrap_cell_text("Supercalifragilistic", 5),
            vec!["Super", "calif", "ragil", "istic"]
        );
    }

    #[test]
    fn wrap_cell_text_never_produces_a_line_wider_than_the_cap() {
        for text in ["a b c d e f g h i j k", "onereallylongwordwithnospaces", ""] {
            for width in 1..8 {
                for line in wrap_cell_text(text, width) {
                    assert!(
                        line.chars().count() <= width,
                        "line {line:?} exceeds width {width} for input {text:?}"
                    );
                }
            }
        }
    }

    /// The counting version must agree with the real wrap on any cell: random
    /// words (some longer than the column), runs of spaces, escaped pipes and
    /// multi-byte characters, at every width a column can shrink to.
    #[test]
    fn wrapped_line_count_matches_wrap_cell_text() {
        use super::super::super::test_support::Rng;
        let pieces = [
            "a",
            "bb",
            "ccc",
            "dddd",
            "eeeee",
            "ffffffff",
            "ggggggggggggg",
            "\\|",
            "x\\|y",
            "ünï",
            "日本語",
            "  ",
            " ",
            "\t",
        ];
        let mut rng = Rng::new(0x7AB1E);
        for _ in 0..20_000 {
            let mut cell = String::new();
            for _ in 0..rng.below(14) {
                cell.push_str(pieces[rng.below(pieces.len())]);
                if rng.below(3) > 0 {
                    cell.push(' ');
                }
            }
            let cell = cell.trim();
            let text = cell.replace("\\|", "|");
            for width in 0..=14 {
                assert_eq!(
                    wrapped_line_count(cell, width),
                    wrap_cell_text(&text, width).len(),
                    "{cell:?} at {width}"
                );
            }
        }
    }

    /// A table that fits unshrunk never wraps a row, so the row pass may skip it.
    #[test]
    fn a_table_that_fits_unshrunk_has_no_wrapped_row() {
        let columns: Vec<Column> = [6, 3, 12].iter().map(|&w| col(w)).collect();
        let rows = [
            "| a | b | c |",
            "| abcdef | abc | abcdefghijkl |",
            "| \\| | x | y |",
        ];
        for budget in [None, Some(10), Some(24), Some(32), Some(60), Some(200)] {
            let widths = resolve_widths(&columns, budget);
            if fits_unshrunk(&columns, budget) {
                assert_eq!(widths, [6, 3, 12], "budget {budget:?}");
                for row in rows {
                    assert_eq!(row_line_count(row, &widths), 1, "{row} at {budget:?}");
                }
            } else {
                assert_ne!(widths, [6, 3, 12], "budget {budget:?}");
            }
        }
    }

    #[test]
    fn row_line_count_matches_the_laid_out_row_height() {
        let columns: Vec<Column> = [6, 3, 12].iter().map(|&w| col(w)).collect();
        let lines = [
            "| a | b | c |",
            "a | b | c",
            "| |",
            "| only one |",
            "| one | two |",
            "| a | b | c | extra | cells |",
            "| a \\| b | c | d |",
            "| Alexandria Fitzgerald Montgomery | x | y |",
            "| Supercalifragilistic | b | tiny |",
            "| ok | long long long long | many words that need to wrap here |",
            "|   spaced     out    |   x   |  y  |",
            "| ünïcödé wörds hëre | b | c |",
            "",
        ];
        for budget in [None, Some(10), Some(24), Some(60)] {
            let widths = resolve_widths(&columns, budget);
            for line in lines {
                assert_eq!(
                    row_line_count(line, &widths),
                    crate::text_editor::table::geometry::row_height(
                        &crate::text_editor::table::geometry::layout_row(line, &widths)
                    ),
                    "line {line:?} budget {budget:?} widths {widths:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;

    const ROWS: [&str; 11] = [
        "| a | b |",
        "|a|b|c",
        "  | x  |  y  |  ",
        "| a \\| b | ünï | 日本 |",
        "| \\| | \\\\| | \\|\\| |",
        "|||",
        "no pipes at all",
        "| tab\t| nbsp\u{a0}|\u{3000}wide\u{3000}|",
        "| vt\u{b}| \u{b}x\u{b} | \r\n| \u{c}y |",
        "",
        "| --- | :-: | --: |",
    ];

    #[test]
    fn a_cell_with_a_formula_is_as_wide_as_the_formula_is_drawn() {
        // #47: the source is 31 characters, the drawn fraction a few.
        let source = "| a | b |\n|---|---|\n| $\\frac{numerator}{denominator}$ | x |";
        let drawn = crate::text_editor::preview::cell_formulas(
            "$\\frac{numerator}{denominator}$",
            &NO_MACROS,
        );
        assert_eq!(drawn.len(), 1);
        let columns = &classify_tables(source).1[0].columns;
        assert_eq!(columns[0].width, drawn[0].1.max(3));
        assert!(columns[0].width < 31, "{columns:?}");
        // One formula is one word, however long its source.
        assert_eq!(columns[0].word, drawn[0].1);
        // The streaming classifier agrees.
        assert_eq!(classify_table_list(source)[0].columns, *columns);
    }

    #[test]
    fn text_around_a_formula_counts_its_words_and_spaces() {
        let drawn = crate::text_editor::preview::cell_formulas("$x$", &NO_MACROS)[0].1;
        // "ab" glued to the formula is one word; the space separates "cd".
        assert_eq!(text_metrics("ab$x$ cd"), (2 + drawn + 1 + 2, 2 + drawn));
        // A formula that cannot be drawn is its source, as before.
        assert_eq!(text_metrics("$\\foo{x}$ y"), ("$\\foo{x}$ y".len(), 9));
    }

    #[test]
    fn cell_word_is_the_longest_word_of_the_cell_text() {
        for line in ROWS {
            let row = scan_row(line);
            let mut words = Vec::new();
            for_each_cell_range(line, |raw| words.push(cell_word(line, raw)));
            let expected: Vec<usize> = row
                .cells
                .iter()
                .map(|c| {
                    c.text
                        .split("<br>")
                        .flat_map(str::split_whitespace)
                        .map(|w| w.chars().count())
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            assert_eq!(words, expected, "{line:?}");
        }
        assert_eq!(longest_word("one<br>three two"), 5);
        assert_eq!(longest_word("a \\| bc"), 2);
        assert_eq!(longest_word("héllo wörld!"), 6);
    }

    #[test]
    fn cell_width_and_cell_count_match_scan_row() {
        for line in ROWS {
            let row = scan_row(line);
            assert_eq!(cell_count(line), row.cells.len(), "{line:?}");
            let mut widths = Vec::new();
            for_each_cell_range(line, |raw| widths.push(cell_width(line, raw)));
            let expected: Vec<usize> = row.cells.iter().map(|c| c.text.chars().count()).collect();
            assert_eq!(widths, expected, "{line:?}");
        }
    }

    #[test]
    fn parse_delimiter_row_matches_the_scan_row_based_answer() {
        let oracle = |line: &str, expected: usize| -> Option<Vec<Align>> {
            let row = scan_row(line);
            if row.cells.len() != expected {
                return None;
            }
            row.cells.iter().map(|c| delimiter_align(&c.text)).collect()
        };
        for line in ROWS {
            for expected in 0..5 {
                assert_eq!(
                    parse_delimiter_row(line, expected),
                    oracle(line, expected),
                    "{line:?} x{expected}"
                );
            }
        }
    }
}

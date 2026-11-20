//! Where a table row's text goes inside its box: for each column, the lines its
//! cell wraps onto, as byte ranges of the row's source line. No pixels and no
//! shaping here, so it is cheap to test; `element.rs` places and paints the
//! lines. Wrapping is the same greedy fill `table_preview::wrapped_line_count`
//! counts (checked against it), so a row's height is the same however it is
//! asked for.

use std::ops::Range;

use super::live_preview::{build_cell_line, raw_cell_line};
use super::table_preview::{Align, LINE_BREAK, for_each_cell_range};

/// Space on each side of a cell's text, in characters. The grid is laid out in
/// characters (the font is monospace); `element.rs` multiplies by the advance.
pub(super) const CELL_PAD: f32 = 1.5;

/// Where `column`'s box starts, in characters from the table's left edge.
pub(super) fn column_left(widths: &[usize], column: usize) -> f32 {
    widths[..column]
        .iter()
        .map(|&w| w as f32 + 2.0 * CELL_PAD)
        .sum()
}

/// Where `column`'s text starts, in characters from the table's left edge.
pub(super) fn text_left(widths: &[usize], column: usize) -> f32 {
    column_left(widths, column) + CELL_PAD
}

/// The table's width in characters, borders included.
pub(super) fn box_width(widths: &[usize]) -> f32 {
    column_left(widths, widths.len())
}

/// The column whose box holds `x` (characters from the table's left edge),
/// clamped to the first and last.
fn column_at(widths: &[usize], x: f32) -> usize {
    (1..widths.len())
        .find(|&c| x < column_left(widths, c))
        .map_or(widths.len().saturating_sub(1), |c| c - 1)
}

/// How far into the spare room of its column a cell's text sits.
pub(super) fn align_fraction(align: Align) -> f32 {
    match align {
        Align::Left => 0.0,
        Align::Center => 0.5,
        Align::Right => 1.0,
    }
}

/// The raw offset in `line` a click lands on: `x` characters from the table's
/// left edge, `row` lines below the row's top. The nearest character of the
/// cell line under it, mapped through what that line shows (hidden markup).
#[cfg(test)]
pub(super) fn offset_at(
    line: &str,
    widths: &[usize],
    aligns: &[Align],
    x: f32,
    row: usize,
) -> usize {
    offset_at_editing(line, widths, aligns, x, row, None)
}

/// [`offset_at`] on a row whose cell `editing` shows its text verbatim.
pub(super) fn offset_at_editing(
    line: &str,
    widths: &[usize],
    aligns: &[Align],
    x: f32,
    row: usize,
    editing: Option<usize>,
) -> usize {
    let column = column_at(widths, x);
    let cell = &layout_row_editing(line, widths, editing)[column];
    let range = cell.lines[row.min(cell.lines.len() - 1)].clone();
    let shown = if editing == Some(column) {
        raw_cell_line(&line[range.clone()])
    } else {
        build_cell_line(&line[range.clone()])
    };
    let chars = shown.display_text.chars().count();
    let spare = widths[column].saturating_sub(chars) as f32;
    let aligned = text_left(widths, column) + spare * align_fraction(aligns[column]);
    let n = ((x - aligned).round().max(0.0) as usize).min(chars);
    let byte = shown
        .display_text
        .char_indices()
        .nth(n)
        .map_or(shown.display_text.len(), |(i, _)| i);
    range.start + shown.raw_offset_for_display(byte)
}

/// The part of a cell that is edited: its raw text without the single space of
/// padding `| a |` conventionally has on each side (`raw` is the cell's whole
/// range between its pipes). Extra spaces are text and stay editable.
pub(super) fn edit_range(line: &str, raw: Range<usize>) -> Range<usize> {
    let cell = &line[raw.clone()];
    if cell == " " {
        return raw.start..raw.start;
    }
    let start = raw.start + usize::from(cell.starts_with(' '));
    let end = raw.end - usize::from(cell.ends_with(' '));
    start..end
}

/// The raw range of every cell of `line`, between its pipes.
pub(super) fn cell_ranges(line: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for_each_cell_range(line, |raw| ranges.push(raw));
    ranges
}

/// The column whose cell holds byte `offset` of `line`: the one it is inside,
/// or the nearest when it is on a pipe or past the ends.
pub(super) fn cell_at(line: &str, offset: usize) -> usize {
    let ranges = cell_ranges(line);
    ranges
        .iter()
        .position(|r| offset <= r.end)
        .unwrap_or(ranges.len().saturating_sub(1))
}

/// Calls `emit` with each line `text` wraps onto when shown verbatim in
/// `width` characters: a break after the last space that fits, else a hard cut.
/// Nothing is dropped, so the ranges are contiguous and cover `text`.
fn raw_cuts(text: &str, width: usize, mut emit: impl FnMut(Range<usize>)) {
    let width = width.max(1);
    let mut start = 0;
    loop {
        let rest = &text[start..];
        let Some((limit, _)) = rest.char_indices().nth(width) else {
            emit(start..text.len());
            return;
        };
        // Spaces at the very end hang past the column rather than wrap: the
        // column is as wide as the trimmed text, and a space just typed at the
        // end must keep the caret on its line.
        if rest[limit..].bytes().all(|b| b == b' ') {
            emit(start..text.len());
            return;
        }
        let cut = match rest[..limit].rfind(' ') {
            Some(space) => start + space + 1,
            None => start + limit,
        };
        emit(start..cut);
        start = cut;
    }
}

/// How many lines [`wrap_raw`] gives `text`.
pub(super) fn raw_line_count(text: &str, width: usize) -> usize {
    let mut count = 0;
    raw_cuts(text, width, |_| count += 1);
    count
}

/// The lines the edited text at `range` of `line` wraps onto in `width`
/// characters, as byte ranges of `line`; never empty.
fn wrap_raw(line: &str, range: Range<usize>, width: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    raw_cuts(&line[range.clone()], width, |r| {
        lines.push(range.start + r.start..range.start + r.end);
    });
    lines
}

/// One column's cell of one row.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CellBox {
    /// The cell's wrapped lines, never empty. Each is a byte range of the
    /// source line covering whole words; a cell with no text has one empty
    /// range at the cell's start.
    pub lines: Vec<Range<usize>>,
}

/// How many lines the row is tall: its tallest cell.
#[cfg(test)]
pub(super) fn row_height(cells: &[CellBox]) -> usize {
    cells.iter().map(|c| c.lines.len()).max().unwrap_or(1)
}

/// Every column's cell of `line` wrapped to `widths` (characters per column). A
/// row with fewer cells than columns gets empty ones at its end; extra cells
/// are ignored, as in `row_line_count`.
#[cfg(test)]
pub(super) fn layout_row(line: &str, widths: &[usize]) -> Vec<CellBox> {
    layout_row_editing(line, widths, None)
}

/// [`layout_row`] for a row whose cell `editing` is being edited: that cell is
/// laid out by [`wrap_raw`].
pub(super) fn layout_row_editing(
    line: &str,
    widths: &[usize],
    editing: Option<usize>,
) -> Vec<CellBox> {
    let mut ranges = Vec::with_capacity(widths.len());
    for_each_cell_range(line, |raw| ranges.push(raw));
    widths
        .iter()
        .enumerate()
        .map(|(column, &width)| match ranges.get(column) {
            Some(raw) if editing == Some(column) => CellBox {
                lines: wrap_raw(line, edit_range(line, raw.clone()), width),
            },
            Some(raw) => CellBox {
                lines: wrap_ranges(line, raw.clone(), width),
            },
            None => CellBox {
                lines: vec![line.len()..line.len()],
            },
        })
        .collect()
}

/// The greedy fill of `table_preview::wrapped_line_count`, returning where each
/// line is instead of how many there are.
fn wrap_ranges(line: &str, raw: Range<usize>, width: usize) -> Vec<Range<usize>> {
    let cell = &line[raw.clone()];
    if !cell.contains(LINE_BREAK) {
        return wrap_ranges_between_breaks(line, raw, width);
    }
    // Each part between `<br>`s wraps on its own; the tags are not shown.
    let mut lines = Vec::new();
    let mut start = raw.start;
    for part in cell.split(LINE_BREAK) {
        let end = start + part.len();
        lines.extend(wrap_ranges_between_breaks(line, start..end, width));
        start = end + LINE_BREAK.len();
    }
    lines
}

fn wrap_ranges_between_breaks(line: &str, raw: Range<usize>, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let cell = &line[raw.clone()];
    let mut lines = Vec::new();
    // The open line: its byte range and how many characters it shows.
    let mut open: Option<(Range<usize>, usize)> = None;
    for (word_start, word) in words(cell) {
        let word_start = raw.start + word_start;
        let mut word = word_start..word_start + word.len();
        let mut len = shown_len(&line[word.clone()]);
        loop {
            let used = open.as_ref().map_or(0, |(_, used)| *used);
            let separator = usize::from(open.is_some());
            if used + separator + len <= width {
                open = Some(match open.take() {
                    Some((range, used)) => (range.start..word.end, used + separator + len),
                    None => (word.clone(), len),
                });
                break;
            }
            if let Some((range, _)) = open.take() {
                lines.push(range);
                continue;
            }
            // Longer than a whole line: cut at `width` characters.
            let cut = word.start + byte_after_shown(&line[word.clone()], width);
            if cut == word.end {
                open = Some((word, len));
                break;
            }
            lines.push(word.start..cut);
            word.start = cut;
            len = shown_len(&line[word.clone()]);
        }
    }
    if let Some((range, _)) = open {
        lines.push(range);
    }
    if lines.is_empty() {
        lines.push(raw.start..raw.start);
    }
    lines
}

/// The whitespace-separated words of `text` with their byte offsets.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_whitespace()
        .map(move |word| (word.as_ptr() as usize - text.as_ptr() as usize, word))
}

/// The byte offset after the first `shown` characters of `word` (all of it if
/// it has no more), counting `\|` as the one character it renders as.
fn byte_after_shown(word: &str, shown: usize) -> usize {
    let bytes = word.as_bytes();
    let (mut at, mut count) = (0, 0);
    while at < word.len() && count < shown {
        at += if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'|') {
            2
        } else {
            word[at..].chars().next().map_or(1, char::len_utf8)
        };
        count += 1;
    }
    at
}

/// Characters `word` takes once `\|` shows as `|`.
fn shown_len(word: &str) -> usize {
    word.chars().count() - word.matches("\\|").count()
}

/// Where byte `offset` of the edited cell `column` of `line` is: the row of the
/// cell it is on, and its x in characters from the table's left edge.
pub(super) fn caret_at(
    line: &str,
    widths: &[usize],
    aligns: &[Align],
    column: usize,
    offset: usize,
) -> (usize, f32) {
    let cell = &layout_row_editing(line, widths, Some(column))[column];
    let row = cell
        .lines
        .iter()
        .position(|r| offset < r.end)
        .unwrap_or(cell.lines.len() - 1);
    (
        row,
        x_in_line(line, widths, aligns, column, &cell.lines[row], offset),
    )
}

/// x in characters of byte `offset` on the edited cell line `range`.
pub(super) fn x_in_line(
    line: &str,
    widths: &[usize],
    aligns: &[Align],
    column: usize,
    range: &Range<usize>,
    offset: usize,
) -> f32 {
    let at = offset.clamp(range.start, range.end);
    let before = line[range.start..at].chars().count();
    let chars = line[range.clone()].chars().count();
    let spare = widths[column].saturating_sub(chars) as f32;
    text_left(widths, column) + spare * align_fraction(aligns[column]) + before as f32
}

/// The byte of the edited cell line `range` nearest `x` characters from the
/// table's left edge.
pub(super) fn offset_in_line(
    line: &str,
    widths: &[usize],
    aligns: &[Align],
    column: usize,
    range: &Range<usize>,
    x: f32,
) -> usize {
    let chars = line[range.clone()].chars().count();
    let spare = widths[column].saturating_sub(chars) as f32;
    let first = text_left(widths, column) + spare * align_fraction(aligns[column]);
    let n = ((x - first).round().max(0.0) as usize).min(chars);
    let byte = line[range.clone()]
        .char_indices()
        .nth(n)
        .map_or(range.len(), |(i, _)| i);
    range.start + byte
}

#[cfg(test)]
mod tests {
    use super::super::table_preview::{row_line_count, row_line_count_editing, wrap_cell_text};
    use super::super::test_support::Rng;
    use super::*;

    fn texts<'a>(line: &'a str, cell: &CellBox) -> Vec<&'a str> {
        cell.lines.iter().map(|r| &line[r.clone()]).collect()
    }

    #[test]
    fn a_cell_that_fits_is_one_line_with_its_own_text() {
        let line = "|  one  | two words |";
        let cells = layout_row(line, &[5, 9]);
        assert_eq!(texts(line, &cells[0]), ["one"]);
        assert_eq!(texts(line, &cells[1]), ["two words"]);
        assert_eq!(row_height(&cells), 1);
    }

    #[test]
    fn a_long_cell_wraps_on_words_and_hard_breaks_a_long_word() {
        let line = "| aa bb cc dd | abcdefghij |";
        let cells = layout_row(line, &[5, 4]);
        assert_eq!(texts(line, &cells[0]), ["aa bb", "cc dd"]);
        assert_eq!(texts(line, &cells[1]), ["abcd", "efgh", "ij"]);
        assert_eq!(row_height(&cells), 3);
    }

    #[test]
    fn missing_cells_are_empty_lines_at_the_end_of_the_row() {
        let line = "| a |";
        let cells = layout_row(line, &[3, 3]);
        assert_eq!(cells[1].lines, [line.len()..line.len()]);
    }

    #[test]
    fn a_click_lands_in_the_cell_it_is_over() {
        let line = "| Alice | **Bob** |";
        let widths = [5, 7];
        let aligns = [Align::Left, Align::Right];
        // Column 0's text starts 1.5 characters in; the nearest boundary to 3.6
        // is after two characters, before the `i`.
        assert_eq!(offset_at(line, &widths, &aligns, 3.6, 0), 4);
        // Left of everything, and right of everything.
        assert_eq!(offset_at(line, &widths, &aligns, -3.0, 0), 2);
        assert_eq!(offset_at(line, &widths, &aligns, 99.0, 0), line.len() - 2);
        // Column 1 is right-aligned (4 spare characters) and `**` is hidden:
        // the `B` shows at 5 + 3 + 1.5 + 4 = 13.5 characters.
        let b = line.find("Bob").unwrap();
        assert_eq!(offset_at(line, &widths, &aligns, 13.6, 0), b);
        assert_eq!(offset_at(line, &widths, &aligns, 15.6, 0), b + 2);
    }

    #[test]
    fn a_line_break_tag_ends_the_line_and_is_not_shown() {
        let line = "| one<br>two three<br><br>x |";
        let cells = layout_row(line, &[20]);
        let shown: Vec<&str> = cells[0]
            .lines
            .iter()
            .map(|r| line[r.clone()].trim())
            .collect();
        assert_eq!(shown, ["one", "two three", "", "x"]);
        assert_eq!(row_line_count(line, &[20]), 4);
        // Narrower: each part wraps on its own.
        assert_eq!(
            row_line_count(line, &[5]),
            layout_row(line, &[5])
                .iter()
                .map(|c| c.lines.len())
                .max()
                .unwrap()
        );
        // Edited, the tag is plain text on one line.
        let edited = layout_row_editing(line, &[40], Some(0));
        assert_eq!(edited[0].lines.len(), 1);
        assert_eq!(row_line_count_editing(line, &[40], Some(0)), 1);
    }

    /// The column is as wide as the cell's trimmed text, so a space typed at the
    /// end of a full cell must not wrap the caret onto a new line.
    #[test]
    fn trailing_spaces_in_an_edited_cell_stay_on_its_line() {
        let aligns = [Align::Left];
        for line in ["| ab  |", "| ab    |", "| a b  |"] {
            let ranges = cell_ranges(line);
            let shown = edit_range(line, ranges[0].clone());
            let (row, _) = caret_at(
                line,
                &[line[shown.clone()].trim().len()],
                &aligns,
                0,
                shown.end,
            );
            assert_eq!(row, 0, "{line:?}");
            let lines = layout_row_editing(line, &[line[shown].trim().len()], Some(0));
            assert_eq!(lines[0].lines.len(), 1, "{line:?}");
        }
    }

    #[test]
    fn columns_are_laid_out_with_padding_on_both_sides() {
        let widths = [5, 3];
        assert_eq!(column_left(&widths, 0), 0.0);
        assert_eq!(text_left(&widths, 0), 1.5);
        assert_eq!(column_left(&widths, 1), 8.0);
        assert_eq!(box_width(&widths), 14.0);
        assert_eq!(column_at(&widths, 7.9), 0);
        assert_eq!(column_at(&widths, 8.0), 1);
        assert_eq!(column_at(&widths, 50.0), 1);
        assert_eq!(column_at(&widths, -4.0), 0);
    }

    /// Same lines, same count as the string-building wrap and the counting one,
    /// on random cells (long words, runs of spaces, escaped pipes, multi-byte).
    #[test]
    fn layout_matches_wrap_cell_text_and_row_line_count() {
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
        ];
        let mut rng = Rng::new(0xB0C5);
        for _ in 0..10_000 {
            let mut line = String::from("|");
            for _ in 0..1 + rng.below(3) {
                for _ in 0..rng.below(12) {
                    line.push_str(pieces[rng.below(pieces.len())]);
                    if rng.below(3) > 0 {
                        line.push(' ');
                    }
                }
                line.push('|');
            }
            let widths: Vec<usize> = (0..3).map(|_| rng.below(12)).collect();
            let cells = layout_row(&line, &widths);
            assert_eq!(
                row_height(&cells),
                row_line_count(&line, &widths),
                "{line:?} {widths:?}"
            );
            let mut ranges = Vec::new();
            for_each_cell_range(&line, |r| ranges.push(r));
            for (c, cell) in cells.iter().enumerate() {
                let expected = match ranges.get(c) {
                    Some(r) => {
                        wrap_cell_text(&line[r.clone()].trim().replace("\\|", "|"), widths[c])
                    }
                    None => vec![String::new()],
                };
                let got: Vec<String> = texts(&line, cell)
                    .iter()
                    .map(|t| {
                        t.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .replace("\\|", "|")
                    })
                    .collect();
                assert_eq!(got, expected, "{line:?} column {c} at {widths:?}");
            }
        }
    }
}

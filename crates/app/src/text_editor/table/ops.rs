//! Structure edits on a table's source lines: insert or delete a row or a
//! column, set a column's alignment. Pure text in, text out: the editor diffs
//! the result against the old lines and replaces only what changed, as one
//! undoable edit.

use super::geometry::cell_ranges;
use super::model::Align;

/// Row 0 of a table is its header and row 1 the delimiter.
const DELIMITER_ROW: usize = 1;

/// What a menu entry or shortcut does to the table around the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::text_editor) enum Op {
    RowAbove,
    RowBelow,
    ColumnLeft,
    ColumnRight,
    DeleteRow,
    DeleteColumn,
    Align(Align),
}

/// The table's lines after an [`Op`] and where the caret goes: `(row, column)`
/// of the cell it should land in.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::text_editor) struct Edited {
    pub lines: Vec<String>,
    pub caret: (usize, usize),
}

/// The raw text of each cell of `line`, padding included.
fn cells(line: &str) -> Vec<String> {
    cell_ranges(line)
        .into_iter()
        .map(|r| line[r].to_string())
        .collect()
}

fn join(cells: &[String]) -> String {
    format!("|{}|", cells.join("|"))
}

/// A row of `columns` empty cells, `| | |`.
pub(in crate::text_editor) fn empty_row(columns: usize) -> String {
    join(&vec![" ".to_string(); columns])
}

/// How many columns the table has: its delimiter row's cells.
fn column_count(lines: &[&str]) -> usize {
    lines.get(DELIMITER_ROW).map_or(0, |l| cell_ranges(l).len())
}

/// Whether `op` can run with the caret in `row` and `column` of `lines`.
pub(in crate::text_editor) fn enabled(lines: &[&str], row: usize, op: Op) -> bool {
    match op {
        Op::RowAbove | Op::DeleteRow => row > DELIMITER_ROW,
        Op::DeleteColumn => column_count(lines) > 1,
        _ => true,
    }
}

/// `op` applied to `lines` with the caret in cell `column` of row `row`;
/// `None` when it does not apply (see [`enabled`]).
pub(in crate::text_editor) fn apply(
    lines: &[&str],
    row: usize,
    column: usize,
    op: Op,
) -> Option<Edited> {
    if !enabled(lines, row, op) {
        return None;
    }
    let columns = column_count(lines);
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    let caret = match op {
        Op::RowAbove => {
            out.insert(row, empty_row(columns));
            (row, column)
        }
        Op::RowBelow => {
            // Below the header is the first body row, after the delimiter.
            let at = if row == 0 { 2 } else { row + 1 };
            out.insert(at, empty_row(columns));
            (at, column)
        }
        Op::DeleteRow => {
            out.remove(row);
            match out.len() {
                len if row < len => (row, column),
                len if len > 2 => (len - 1, column),
                _ => (0, column),
            }
        }
        Op::ColumnLeft | Op::ColumnRight => {
            let at = column + usize::from(op == Op::ColumnRight);
            for (i, line) in out.iter_mut().enumerate() {
                let mut parts = cells(line);
                while parts.len() < at {
                    parts.push(" ".to_string());
                }
                parts.insert(at, if i == DELIMITER_ROW { "---" } else { " " }.to_string());
                *line = join(&parts);
            }
            (row, at)
        }
        Op::DeleteColumn => {
            for line in &mut out {
                let mut parts = cells(line);
                if column < parts.len() {
                    parts.remove(column);
                    *line = join(&parts);
                }
            }
            (row, column.min(columns - 2))
        }
        Op::Align(align) => {
            let mut parts = cells(&out[DELIMITER_ROW]);
            if let Some(part) = parts.get_mut(column) {
                *part = aligned_delimiter(part, align);
                out[DELIMITER_ROW] = join(&parts);
            }
            (row, column)
        }
    };
    Some(Edited { lines: out, caret })
}

/// Moves body line `from` to just before line `before` (`before` may be one
/// past the last line). `None` for the header or delimiter, or a move that
/// changes nothing. The caret goes to cell `column` of the moved row.
pub(in crate::text_editor) fn move_row(
    lines: &[&str],
    from: usize,
    before: usize,
    column: usize,
) -> Option<Edited> {
    if from <= DELIMITER_ROW || from >= lines.len() || before <= DELIMITER_ROW {
        return None;
    }
    if before == from || before == from + 1 || before > lines.len() {
        return None;
    }
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    let moved = out.remove(from);
    let to = if before > from { before - 1 } else { before };
    out.insert(to, moved);
    Some(Edited {
        lines: out,
        caret: (to, column),
    })
}

/// Moves column `from` to just before column `before` (`before` may be the
/// column count), in every row including the delimiter, so its alignment
/// travels with it. `None` for a move that changes nothing.
pub(in crate::text_editor) fn move_column(
    lines: &[&str],
    from: usize,
    before: usize,
    row: usize,
) -> Option<Edited> {
    let columns = column_count(lines);
    if from >= columns || before > columns || before == from || before == from + 1 {
        return None;
    }
    let to = if before > from { before - 1 } else { before };
    let out = lines
        .iter()
        .map(|line| {
            let mut parts = cells(line);
            while parts.len() < columns {
                parts.push(" ".to_string());
            }
            let moved = parts.remove(from);
            parts.insert(to, moved);
            join(&parts)
        })
        .collect();
    Some(Edited {
        lines: out,
        caret: (row, to),
    })
}

/// A delimiter cell with the same dashes and padding as `cell`, marked for `align`.
fn aligned_delimiter(cell: &str, align: Align) -> String {
    let dashes = "-".repeat(cell.matches('-').count().max(3));
    let (lead, trail) = (
        &cell[..cell.len() - cell.trim_start().len()],
        &cell[cell.trim_end().len()..],
    );
    let mark = match align {
        Align::Left => format!(":{dashes}"),
        Align::Center => format!(":{dashes}:"),
        Align::Right => format!("{dashes}:"),
    };
    format!("{lead}{mark}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: [&str; 4] = ["| a | b |", "|---|---|", "| c | d |", "| e | f |"];

    fn run(row: usize, column: usize, op: Op) -> Edited {
        apply(&T, row, column, op).expect("applies")
    }

    fn lines(edited: &Edited) -> Vec<&str> {
        edited.lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_row_goes_above_or_below_the_current_one() {
        let above = run(3, 1, Op::RowAbove);
        assert_eq!(lines(&above), [T[0], T[1], T[2], "| | |", T[3]]);
        assert_eq!(above.caret, (3, 1));
        let below = run(2, 0, Op::RowBelow);
        assert_eq!(lines(&below), [T[0], T[1], T[2], "| | |", T[3]]);
        assert_eq!(below.caret, (3, 0));
    }

    #[test]
    fn a_row_below_the_header_is_the_first_body_row() {
        let edited = run(0, 1, Op::RowBelow);
        assert_eq!(lines(&edited), [T[0], T[1], "| | |", T[2], T[3]]);
        assert_eq!(edited.caret, (2, 1));
    }

    #[test]
    fn the_header_and_delimiter_cannot_be_deleted_or_have_a_row_above() {
        for row in [0, 1] {
            assert!(apply(&T, row, 0, Op::DeleteRow).is_none());
            assert!(apply(&T, row, 0, Op::RowAbove).is_none());
        }
    }

    #[test]
    fn deleting_a_row_keeps_the_caret_on_the_same_column() {
        let middle = run(2, 1, Op::DeleteRow);
        assert_eq!(lines(&middle), [T[0], T[1], T[3]]);
        assert_eq!(middle.caret, (2, 1));
        let last = run(3, 0, Op::DeleteRow);
        assert_eq!(lines(&last), [T[0], T[1], T[2]]);
        assert_eq!(last.caret, (2, 0));
        let only = apply(&T[..3], 2, 1, Op::DeleteRow).unwrap();
        assert_eq!(lines(&only), [T[0], T[1]]);
        assert_eq!(only.caret, (0, 1), "no body row left: the header");
    }

    #[test]
    fn a_column_goes_left_or_right_with_a_dash_cell_in_the_delimiter() {
        let left = run(2, 0, Op::ColumnLeft);
        assert_eq!(
            lines(&left),
            ["| | a | b |", "|---|---|---|", "| | c | d |", "| | e | f |"]
        );
        assert_eq!(left.caret, (2, 0));
        let right = run(2, 0, Op::ColumnRight);
        assert_eq!(
            lines(&right),
            ["| a | | b |", "|---|---|---|", "| c | | d |", "| e | | f |"]
        );
        assert_eq!(right.caret, (2, 1));
    }

    #[test]
    fn a_column_op_pads_a_short_row_and_writes_the_missing_pipes() {
        let ragged = ["| a | b |", "|---|---|", "| c", "d | e"];
        let edited = apply(&ragged, 2, 1, Op::ColumnRight).unwrap();
        assert_eq!(
            lines(&edited),
            ["| a | b | |", "|---|---|---|", "| c| | |", "|d | e| |"]
        );
    }

    #[test]
    fn deleting_a_column_removes_it_from_every_row() {
        let edited = run(2, 0, Op::DeleteColumn);
        assert_eq!(lines(&edited), ["| b |", "|---|", "| d |", "| f |"]);
        assert_eq!(edited.caret, (2, 0));
        let last = run(2, 1, Op::DeleteColumn);
        assert_eq!(lines(&last), ["| a |", "|---|", "| c |", "| e |"]);
        assert_eq!(last.caret, (2, 0));
        assert!(apply(&["| a |", "|---|"], 0, 0, Op::DeleteColumn).is_none());
    }

    #[test]
    fn a_body_row_moves_to_before_another_and_the_header_stays() {
        let up = move_row(&T, 3, 2, 1).unwrap();
        assert_eq!(lines(&up), [T[0], T[1], T[3], T[2]]);
        assert_eq!(up.caret, (2, 1));
        let down = move_row(&T, 2, 4, 0).unwrap();
        assert_eq!(lines(&down), [T[0], T[1], T[3], T[2]]);
        assert_eq!(down.caret, (3, 0));
        for (from, before) in [(2, 2), (2, 3), (0, 3), (1, 3), (3, 1), (3, 0), (9, 2)] {
            assert!(
                move_row(&T, from, before, 0).is_none(),
                "{from} -> {before}"
            );
        }
    }

    #[test]
    fn a_column_moves_with_its_alignment() {
        let aligned = ["| a | b | c |", "|:--|---|--:|", "| 1 | 2 | 3 |"];
        let first_to_end = move_column(&aligned, 0, 3, 2).unwrap();
        assert_eq!(
            lines(&first_to_end),
            ["| b | c | a |", "|---|--:|:--|", "| 2 | 3 | 1 |"]
        );
        assert_eq!(first_to_end.caret, (2, 2));
        let last_to_start = move_column(&aligned, 2, 0, 0).unwrap();
        assert_eq!(lines(&last_to_start)[0], "| c | a | b |");
        for (from, before) in [(1, 1), (1, 2), (3, 0), (0, 4)] {
            assert!(
                move_column(&aligned, from, before, 0).is_none(),
                "{from} -> {before}"
            );
        }
    }

    #[test]
    fn alignment_rewrites_only_that_columns_delimiter_cell() {
        let center = run(2, 1, Op::Align(Align::Center));
        assert_eq!(lines(&center), [T[0], "|---|:---:|", T[2], T[3]]);
        assert_eq!(center.caret, (2, 1));
        let spaced = ["| a |", "| ----- |", "| c |"];
        let right = apply(&spaced, 0, 0, Op::Align(Align::Right)).unwrap();
        assert_eq!(lines(&right)[1], "| -----: |");
        let left = apply(&spaced, 0, 0, Op::Align(Align::Left)).unwrap();
        assert_eq!(lines(&left)[1], "| :----- |");
    }
}

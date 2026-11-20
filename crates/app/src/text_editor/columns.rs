//! Side-by-side columns: a region of ordinary Markdown shown in 2 to 4
//! columns (design in `ARCHITECTURE.md`, "Side-by-side columns").
//!
//! ```text
//! ::: columns 60/40
//!
//! Left text.
//!
//! ---
//!
//! Right text.
//!
//! :::
//! ```
//!
//! Every marker (`::: columns [ratio]`, `---`, `:::`) is alone on a line at
//! column 0 with a blank line before and after, so another renderer reads the
//! region as plain text with a rule between the parts. A marker that breaks
//! that rule is ordinary text, and so is every marker inside fenced code.
//! Regions do not nest: a second open marker abandons the first (the last open
//! wins), and the first `:::` closes the region. A region that is unclosed,
//! has more than [`MAX_COLUMNS`] columns, or whose ratio does not fit its
//! column count is not a region at all.
//!
//! This module is pure line scanning; colours, widths in pixels and painting
//! live elsewhere.
// Wired into `DocumentCache` in the next step.
#![allow(dead_code)]

use std::ops::Range;

pub const MAX_COLUMNS: usize = 4;

const OPEN: &str = "::: columns";
const CLOSE: &str = ":::";
const SEPARATOR: &str = "---";

/// One recognised region, by line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    /// The open marker's line.
    pub start_line: usize,
    /// One past the close marker's line.
    pub end_line: usize,
    /// How many columns (2 to [`MAX_COLUMNS`]).
    pub count: usize,
    /// Each column's share in percent; the first `count` are used and sum to 100.
    pub percent: [u8; MAX_COLUMNS],
    /// The separator lines; the first `count - 1` are used.
    separators: [usize; MAX_COLUMNS - 1],
}

impl Columns {
    /// The separator lines between the columns, in order.
    pub fn separators(&self) -> &[usize] {
        &self.separators[..self.count - 1]
    }

    /// The close marker's line.
    pub fn close_line(&self) -> usize {
        self.end_line - 1
    }

    /// Whether `line` is one of the region's three kinds of marker line.
    pub fn is_marker(&self, line: usize) -> bool {
        line == self.start_line || line == self.close_line() || self.separators().contains(&line)
    }

    /// The lines of column `k`, without the blank line that pads each marker
    /// (empty when only that blank line lies between two markers).
    pub fn column_lines(&self, k: usize) -> Range<usize> {
        assert!(k < self.count, "column {k} of {}", self.count);
        let first = if k == 0 {
            self.start_line
        } else {
            self.separators[k - 1]
        };
        let next = if k + 1 == self.count {
            self.close_line()
        } else {
            self.separators[k]
        };
        let (start, end) = (first + 2, next - 1);
        if start < end {
            start..end
        } else {
            start..start
        }
    }
}

#[derive(Clone, Copy)]
enum Marker {
    /// `::: columns`, with its explicit ratio if one was written.
    Open(Option<Ratio>),
    Separator,
    Close,
}

#[derive(Clone, Copy)]
struct Ratio {
    percent: [u8; MAX_COLUMNS],
    count: usize,
}

/// A region whose open marker has been seen.
struct Pending {
    start: usize,
    ratio: Option<Ratio>,
    separators: [usize; MAX_COLUMNS - 1],
    seen: usize,
}

impl Pending {
    /// The finished region, or `None` when the ratio and the column count
    /// disagree.
    fn close(self, line: usize) -> Option<Columns> {
        let count = self.seen + 1;
        let percent = match self.ratio {
            Some(r) if r.count == count => r.percent,
            Some(_) => return None,
            None => equal_shares(count),
        };
        Some(Columns {
            start_line: self.start,
            end_line: line + 1,
            count,
            percent,
            separators: self.separators,
        })
    }
}

/// `100 / count` each, the remainder going to the last column.
fn equal_shares(count: usize) -> [u8; MAX_COLUMNS] {
    assert!((2..=MAX_COLUMNS).contains(&count));
    let each = (100 / count) as u8;
    let mut percent = [0; MAX_COLUMNS];
    percent[..count].fill(each);
    percent[count - 1] += 100 - each * count as u8;
    percent
}

/// What marker `line` is, if it is one. Layout (blank lines around it) is
/// checked by the caller.
fn marker_of(line: &str) -> Option<Marker> {
    let line = line.trim_end();
    if line == SEPARATOR {
        return Some(Marker::Separator);
    }
    if line == CLOSE {
        return Some(Marker::Close);
    }
    let rest = line.strip_prefix(OPEN)?;
    if rest.is_empty() {
        return Some(Marker::Open(None));
    }
    parse_ratio(rest.strip_prefix(' ')?.trim_start()).map(|r| Marker::Open(Some(r)))
}

/// `60/40`, `20/50/30`: 2 to [`MAX_COLUMNS`] whole percents of at least 1,
/// summing to 100.
fn parse_ratio(text: &str) -> Option<Ratio> {
    let mut percent = [0u8; MAX_COLUMNS];
    let mut count = 0;
    for part in text.split('/') {
        // `parse` alone would also take a leading `+`.
        if count == MAX_COLUMNS
            || part.len() > 3
            || part.is_empty()
            || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let value: u8 = part.parse().ok().filter(|&v| v >= 1)?;
        percent[count] = value;
        count += 1;
    }
    let sum: u32 = percent[..count].iter().map(|&p| u32::from(p)).sum();
    (count >= 2 && sum == 100).then_some(Ratio { percent, count })
}

/// Every region of a document of `line_count` lines, in order. `line` gives
/// a line's text without its newline; `in_code` is true for the lines of a
/// fenced code block, fences included.
pub fn scan<'a>(
    line_count: usize,
    line: impl Fn(usize) -> &'a str,
    in_code: impl Fn(usize) -> bool,
) -> Vec<Columns> {
    let is_blank = |i: usize| line(i).trim().is_empty();
    let isolated =
        |i: usize| (i == 0 || is_blank(i - 1)) && (i + 1 == line_count || is_blank(i + 1));
    let mut regions = Vec::new();
    let mut pending: Option<Pending> = None;
    for i in 0..line_count {
        let text = line(i);
        // Most lines are neither; this skips the rest of the work for them.
        if !matches!(text.as_bytes().first(), Some(b':' | b'-')) || in_code(i) {
            continue;
        }
        let Some(marker) = marker_of(text).filter(|_| isolated(i)) else {
            continue;
        };
        match marker {
            Marker::Open(ratio) => {
                pending = Some(Pending {
                    start: i,
                    ratio,
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            Marker::Separator => {
                if let Some(p) = pending.as_mut() {
                    if p.seen == MAX_COLUMNS - 1 {
                        pending = None;
                    } else {
                        p.separators[p.seen] = i;
                        p.seen += 1;
                    }
                }
            }
            Marker::Close => {
                // A region needs a separator: one column is not columns.
                if let Some(p) = pending.take().filter(|p| p.seen >= 1) {
                    regions.extend(p.close(i));
                }
            }
        }
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_text(text: &str) -> Vec<Columns> {
        let lines: Vec<&str> = text.lines().collect();
        scan(lines.len(), |i| lines[i], |_| false)
    }

    const TWO: &str = "::: columns\n\nleft\n\n---\n\nright\n\n:::\n";

    #[test]
    fn two_equal_columns() {
        let regions = scan_text(TWO);
        assert_eq!(regions.len(), 1);
        let r = regions[0];
        assert_eq!((r.start_line, r.end_line, r.count), (0, 9, 2));
        assert_eq!(&r.percent[..2], &[50, 50]);
        assert_eq!(r.separators(), &[4]);
        assert_eq!(r.close_line(), 8);
        assert_eq!(r.column_lines(0), 2..3);
        assert_eq!(r.column_lines(1), 6..7);
        assert!([0, 4, 8].iter().all(|&l| r.is_marker(l)));
        assert!(!r.is_marker(2));
    }

    #[test]
    fn ratio_sets_the_shares() {
        let r = scan_text("::: columns 20/50/30\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::")[0];
        assert_eq!(r.count, 3);
        assert_eq!(&r.percent[..3], &[20, 50, 30]);
    }

    #[test]
    fn equal_shares_give_the_remainder_to_the_last() {
        let r = scan_text("::: columns\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::")[0];
        assert_eq!(&r.percent[..3], &[33, 33, 34]);
    }

    #[test]
    fn bad_ratios_make_no_region() {
        for ratio in [
            "60/30",
            "60/40/0",
            "100",
            "a/b",
            "60/40/",
            "20/20/20/20/20",
            "60/-40",
            "+60/40",
        ] {
            let text = format!("::: columns {ratio}\n\na\n\n---\n\nb\n\n:::");
            assert!(scan_text(&text).is_empty(), "{ratio}");
        }
    }

    #[test]
    fn ratio_count_must_match_columns() {
        let text = "::: columns 60/40\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::";
        assert!(scan_text(text).is_empty());
    }

    #[test]
    fn markers_need_blank_lines_around_them() {
        // No blank before the separator: a heading underline elsewhere.
        assert!(scan_text("::: columns\n\nleft\n---\n\nright\n\n:::").is_empty());
        // No blank after the open marker.
        assert!(scan_text("::: columns\nleft\n\n---\n\nright\n\n:::").is_empty());
        // No blank before the close marker.
        assert!(scan_text("::: columns\n\nleft\n\n---\n\nright\n:::").is_empty());
    }

    #[test]
    fn markers_must_be_at_column_zero_and_exact() {
        assert!(scan_text(" ::: columns\n\na\n\n---\n\nb\n\n:::").is_empty());
        assert!(scan_text("::: columnsx\n\na\n\n---\n\nb\n\n:::").is_empty());
        assert!(scan_text("::: columns\n\na\n\n----\n\nb\n\n:::").is_empty());
        assert!(scan_text("> ::: columns\n>\n> a\n>\n> ---\n>\n> b\n>\n> :::").is_empty());
        // Trailing spaces are fine.
        assert_eq!(
            scan_text("::: columns  \n\na\n\n---  \n\nb\n\n:::  ").len(),
            1
        );
    }

    #[test]
    fn unclosed_or_single_column_is_not_a_region() {
        assert!(scan_text("::: columns\n\na\n\n---\n\nb\n").is_empty());
        assert!(scan_text("::: columns\n\na\n\n:::").is_empty());
        assert!(scan_text("---\n\n:::").is_empty());
    }

    #[test]
    fn more_than_four_columns_is_not_a_region() {
        let mut text = String::from("::: columns\n\n");
        for _ in 0..4 {
            text.push_str("a\n\n---\n\n");
        }
        text.push_str("a\n\n:::");
        assert!(scan_text(&text).is_empty());
        let r = scan_text(&text.replacen("---\n\na\n\n", "", 1))[0];
        assert_eq!(r.count, 4);
    }

    #[test]
    fn the_last_open_wins_and_regions_do_not_nest() {
        let text = "::: columns\n\nlost\n\n::: columns\n\na\n\n---\n\nb\n\n:::";
        let regions = scan_text(text);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].start_line, 4);
    }

    #[test]
    fn several_regions_in_order() {
        let text = format!("{TWO}\ntext\n\n{TWO}");
        let regions = scan_text(&text);
        assert_eq!(regions.len(), 2);
        assert!(regions[0].end_line <= regions[1].start_line);
    }

    #[test]
    fn empty_columns_are_allowed() {
        let r = scan_text("::: columns\n\n---\n\n:::")[0];
        assert_eq!(r.column_lines(0), 2..2);
        assert_eq!(r.column_lines(1), 4..4);
    }

    #[test]
    fn code_fences_hide_markers() {
        let text = "::: columns\n\na\n\n---\n\nb\n\n:::";
        let lines: Vec<&str> = text.lines().collect();
        // The separator is inside a fence: no second column.
        assert!(scan(lines.len(), |i| lines[i], |i| i == 4).is_empty());
    }

    #[test]
    fn a_document_end_counts_as_blank() {
        assert_eq!(scan_text("::: columns\n\na\n\n---\n\nb\n\n:::").len(), 1);
        assert!(scan_text("").is_empty());
    }
}

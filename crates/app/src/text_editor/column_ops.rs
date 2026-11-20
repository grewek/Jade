//! Structure edits on a columns region, as pure text in, text out: the lines of
//! the whole region (open marker to close marker) go in, the lines that replace
//! them come out, so the editor can diff them into one undo step. The pixel
//! side (which divider is dragged, where) is `column_handles.rs`.

use super::columns::{Columns, MAX_COLUMNS, equal_shares};

/// The smallest share a column keeps when a divider is dragged.
pub const MIN_SHARE: u8 = 10;
/// Shares snap to multiples of this.
pub const SNAP: u8 = 5;

/// The open marker for `percent`: bare when the shares are the equal ones a
/// bare marker means, else with the written ratio.
pub fn open_marker(percent: &[u8]) -> String {
    assert!((2..=MAX_COLUMNS).contains(&percent.len()));
    if percent == &equal_shares(percent.len())[..percent.len()] {
        return "::: columns".to_owned();
    }
    let ratio: Vec<String> = percent.iter().map(u8::to_string).collect();
    format!("::: columns {}", ratio.join("/"))
}

/// The region with its shares set to `percent`.
pub fn with_ratio(lines: &[&str], percent: &[u8]) -> Vec<String> {
    assert!(lines.len() >= 3);
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
    out[0] = open_marker(percent);
    out
}

/// The region with an empty column added at the right, the shares equal again.
/// `None` at the column limit.
pub fn with_added_column(lines: &[&str], region: &Columns) -> Option<Vec<String>> {
    assert_eq!(lines.len(), region.end_line - region.start_line);
    if region.count >= MAX_COLUMNS {
        return None;
    }
    let equal = equal_shares(region.count + 1);
    let mut out = with_ratio(lines, &equal[..=region.count]);
    // The close marker's padding blank line already ends the last column, and
    // the new separator is padded by it and by one more.
    let close = out.len() - 1;
    out.splice(close..close, ["---".to_owned(), String::new()]);
    Some(out)
}

/// The region's columns one after another, without markers; empty columns drop.
pub fn unwrapped(lines: &[&str], region: &Columns) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for k in 0..region.count {
        let range = region.column_lines(k);
        if range.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(String::new());
        }
        let (a, b) = (
            range.start - region.start_line,
            range.end - region.start_line,
        );
        out.extend(lines[a..b].iter().map(|l| (*l).to_owned()));
    }
    out
}

/// `block` as the first of two columns, the second empty.
pub fn wrapped(block: &[&str]) -> Vec<String> {
    assert!(!block.is_empty());
    let mut out = Vec::with_capacity(block.len() + 6);
    out.extend(["::: columns".to_owned(), String::new()]);
    out.extend(block.iter().map(|l| (*l).to_owned()));
    out.extend(["", "---", "", ":::"].map(str::to_owned));
    out
}

/// The shares after the divider between columns `k` and `k + 1` is dragged so
/// column `k` takes `share` percent: snapped, neither side under [`MIN_SHARE`],
/// the pair's total kept.
pub fn dragged(percent: &[u8], k: usize, share: f32) -> Vec<u8> {
    assert!(k + 1 < percent.len());
    let pair = percent[k] + percent[k + 1];
    assert!(pair >= 2 * MIN_SHARE);
    let snapped = (share / f32::from(SNAP)).round() as i32 * i32::from(SNAP);
    let first = snapped.clamp(i32::from(MIN_SHARE), i32::from(pair - MIN_SHARE)) as u8;
    let mut out = percent.to_vec();
    (out[k], out[k + 1]) = (first, pair - first);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_editor::columns::scan;

    fn region(text: &str) -> (Vec<&str>, Columns) {
        let lines: Vec<&str> = text.split('\n').collect();
        let found = scan(lines.iter().map(|l| (*l, false)));
        assert_eq!(found.len(), 1, "one region in {text:?}");
        (lines, found[0])
    }

    fn scan_one(lines: &[String]) -> Columns {
        let found = scan(lines.iter().map(|l| (l.as_str(), false)));
        assert_eq!(found.len(), 1, "one region in {lines:?}");
        found[0]
    }

    #[test]
    fn a_bare_marker_means_equal_shares() {
        assert_eq!(open_marker(&[50, 50]), "::: columns");
        assert_eq!(open_marker(&[33, 33, 34]), "::: columns");
        assert_eq!(open_marker(&[60, 40]), "::: columns 60/40");
    }

    #[test]
    fn a_ratio_is_read_back_as_written() {
        let (lines, _) = region("::: columns\n\na\n\n---\n\nb\n\n:::");
        let out = with_ratio(&lines, &[65, 35]);
        assert_eq!(scan_one(&out).percent[..2], [65, 35]);
    }

    #[test]
    fn an_added_column_is_a_region_with_one_more_empty_column() {
        let (lines, r) = region("::: columns 60/40\n\na\n\n---\n\nb\n\n:::");
        let out = with_added_column(&lines, &r).unwrap();
        let grown = scan_one(&out);
        assert_eq!(grown.count, 3);
        assert_eq!(grown.percent[..3], [33, 33, 34]);
        assert!(grown.column_lines(2).is_empty());
        assert_eq!(out[2], "a");
        assert_eq!(out[6], "b");
    }

    #[test]
    fn no_column_is_added_past_the_limit() {
        let (lines, r) = region("::: columns\n\n1\n\n---\n\n2\n\n---\n\n3\n\n---\n\n4\n\n:::");
        assert_eq!(r.count, 4);
        assert!(with_added_column(&lines, &r).is_none());
    }

    #[test]
    fn unwrapping_stacks_the_columns_and_drops_the_empty_ones() {
        let (lines, r) = region("::: columns\n\n---\n\nx\ny\n\n---\n\nz\n\n:::");
        assert_eq!(unwrapped(&lines, &r), ["x", "y", "", "z"]);
    }

    #[test]
    fn a_wrapped_block_is_a_region_with_an_empty_second_column() {
        let out = wrapped(&["a", "b"]);
        let r = scan_one(&out);
        assert_eq!(r.count, 2);
        assert_eq!(r.column_lines(0), 2..4);
        assert!(r.column_lines(1).is_empty());
    }

    #[test]
    fn a_dragged_divider_snaps_and_keeps_the_pair() {
        assert_eq!(dragged(&[50, 50], 0, 62.), [60, 40]);
        assert_eq!(dragged(&[50, 50], 0, 3.), [10, 90]);
        assert_eq!(dragged(&[50, 50], 0, 99.), [90, 10]);
        assert_eq!(dragged(&[20, 50, 30], 1, 30.), [20, 30, 50]);
        assert_eq!(dragged(&[33, 33, 34], 0, 40.), [40, 26, 34]);
    }
}

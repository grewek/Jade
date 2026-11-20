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
const OPEN_ASIDE: &str = "::: aside";
/// An aside's float share, in percent of the line (without the gap).
pub const ASIDE_MIN: u8 = 10;
pub const ASIDE_MAX: u8 = 90;
pub const ASIDE_DEFAULT: u8 = 40;
const CLOSE: &str = ":::";
const SEPARATOR: &str = "---";

/// What a region lays out as: equal-height columns, or a block floated to one
/// side with the following text flowing around it (`::: aside right 40%`).
/// An aside is a two-part region (the floated block, `---`, the text), its
/// `percent` being `[share, 100 - share]` of the float and the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Columns,
    Aside { right: bool },
}

/// One recognised region, by line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub kind: Kind,
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
    /// Whether this is a floated block with text around it.
    pub fn is_aside(&self) -> bool {
        matches!(self.kind, Kind::Aside { .. })
    }

    /// The separator lines between the columns, in order.
    pub fn separators(&self) -> &[usize] {
        &self.separators[..self.count - 1]
    }

    /// The close marker's line.
    pub fn close_line(&self) -> usize {
        self.end_line - 1
    }

    /// The column whose content lines include `line`; `None` for a marker or
    /// a padding blank line.
    pub fn column_of(&self, line: usize) -> Option<usize> {
        (0..self.count).find(|&k| self.column_lines(k).contains(&line))
    }

    /// Whether `line` is the open or the close marker, the two that render as
    /// nothing (a separator renders as the usual rule).
    pub fn is_frame(&self, line: usize) -> bool {
        line == self.start_line || line == self.close_line()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// `::: columns`, with its explicit ratio if one was written.
    Open(Option<Ratio>),
    /// `::: aside [left|right] [N%]`.
    OpenAside(AsideSpec),
    Separator,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsideSpec {
    pub right: bool,
    pub share: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    percent: [u8; MAX_COLUMNS],
    count: usize,
}

/// A region whose open marker has been seen.
struct Pending {
    start: usize,
    ratio: Option<Ratio>,
    aside: Option<AsideSpec>,
    separators: [usize; MAX_COLUMNS - 1],
    seen: usize,
}

impl Pending {
    /// The finished region, or `None` when the ratio and the column count
    /// disagree.
    fn close(self, line: usize) -> Option<Columns> {
        let count = self.seen + 1;
        if let Some(AsideSpec { right, share }) = self.aside {
            // A float and its text: exactly two parts.
            if count != 2 {
                return None;
            }
            let mut percent = [0; MAX_COLUMNS];
            (percent[0], percent[1]) = (share, 100 - share);
            return Some(Columns {
                kind: Kind::Aside { right },
                start_line: self.start,
                end_line: line + 1,
                count,
                percent,
                separators: self.separators,
            });
        }
        let percent = match self.ratio {
            Some(r) if r.count == count => r.percent,
            Some(_) => return None,
            None => equal_shares(count),
        };
        Some(Columns {
            kind: Kind::Columns,
            start_line: self.start,
            end_line: line + 1,
            count,
            percent,
            separators: self.separators,
        })
    }
}

/// `100 / count` each, the remainder going to the last column.
pub fn equal_shares(count: usize) -> [u8; MAX_COLUMNS] {
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
    if let Some(rest) = line.strip_prefix(OPEN_ASIDE) {
        return parse_aside(rest).map(Marker::OpenAside);
    }
    let rest = line.strip_prefix(OPEN)?;
    if rest.is_empty() {
        return Some(Marker::Open(None));
    }
    parse_ratio(rest.strip_prefix(' ')?.trim_start()).map(|r| Marker::Open(Some(r)))
}

/// What follows `::: aside`: nothing, or `left`/`right` and a share (`40` or
/// `40%`) in either order, each at most once. Right and 40% by default.
fn parse_aside(rest: &str) -> Option<AsideSpec> {
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let (mut side, mut share) = (None, None);
    for word in rest.split_whitespace() {
        match word {
            "left" | "right" if side.is_none() => side = Some(word == "right"),
            _ if share.is_none() => {
                let digits = word.strip_suffix('%').unwrap_or(word);
                if digits.is_empty()
                    || digits.len() > 2
                    || !digits.bytes().all(|b| b.is_ascii_digit())
                {
                    return None;
                }
                let value: u8 = digits.parse().ok()?;
                share = Some(value).filter(|v| (ASIDE_MIN..=ASIDE_MAX).contains(v));
                share?;
            }
            _ => return None,
        }
    }
    Some(AsideSpec {
        right: side.unwrap_or(true),
        share: share.unwrap_or(ASIDE_DEFAULT),
    })
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

/// A marker and the line it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub line: usize,
    pub marker: Marker,
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The markers among `lines`, the first of which is line `first_line`; each
/// item is the line's text and whether it is in fenced code (fences
/// included). `before_blank` and `after_blank` say whether the line before the
/// window and the line after it are blank (true at a document's edge), which a
/// marker at the window's own edge needs to know it is isolated.
pub fn place_markers<'a>(
    first_line: usize,
    lines: impl Iterator<Item = (&'a str, bool)>,
    before_blank: bool,
    after_blank: bool,
) -> Vec<Placed> {
    let mut placed = Vec::new();
    let mut lines = lines.peekable();
    let mut prev_blank = before_blank;
    let mut line = first_line;
    while let Some((text, in_code)) = lines.next() {
        // Most lines are not markers; this skips the rest of the work for them.
        if matches!(text.as_bytes().first(), Some(b':' | b'-'))
            && !in_code
            && let Some(marker) = marker_of(text)
            && prev_blank
            && lines
                .peek()
                .map_or(after_blank, |&(next, _)| is_blank(next))
        {
            placed.push(Placed { line, marker });
        }
        prev_blank = is_blank(text);
        line += 1;
    }
    placed
}

/// The regions the markers form, in order (see the module comment for the
/// rules). `markers` must be sorted by line.
pub fn derive(markers: &[Placed]) -> Vec<Columns> {
    let mut regions = Vec::new();
    let mut pending: Option<Pending> = None;
    for &Placed { line, marker } in markers {
        match marker {
            Marker::Open(ratio) => {
                pending = Some(Pending {
                    start: line,
                    ratio,
                    aside: None,
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            Marker::OpenAside(spec) => {
                pending = Some(Pending {
                    start: line,
                    ratio: None,
                    aside: Some(spec),
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            Marker::Separator => {
                if let Some(p) = pending.as_mut() {
                    if p.seen == MAX_COLUMNS - 1 {
                        pending = None;
                    } else {
                        p.separators[p.seen] = line;
                        p.seen += 1;
                    }
                }
            }
            Marker::Close => {
                // A region needs a separator: one column is not columns.
                if let Some(p) = pending.take().filter(|p| p.seen >= 1) {
                    regions.extend(p.close(line));
                }
            }
        }
    }
    regions
}

/// Every region of a document, `lines` being as for [`place_markers`]. The
/// oracle the cache's incremental markers are tested against.
pub fn scan<'a>(lines: impl Iterator<Item = (&'a str, bool)>) -> Vec<Columns> {
    derive(&place_markers(0, lines, true, true))
}

/// Space between two columns, in pixels; the divider is drawn in its middle.
pub const GAP: f32 = 24.;
/// A column narrower than this makes the whole region stack (about 20
/// characters at the editor's size).
pub const MIN_COLUMN: f32 = 160.;

/// Where the columns of one region sit across a line `total` pixels wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub count: usize,
    x: [f32; MAX_COLUMNS],
    width: [f32; MAX_COLUMNS],
    /// For an aside: whether the float is on the right. Part 0 is the float
    /// and part 1 the text beside it (its width is the channel; the text
    /// below the float uses the whole line, see `total`).
    pub aside: Option<bool>,
    /// The line's whole width.
    pub total: f32,
}

impl Split {
    /// Column `k`'s left edge, from the line's left edge.
    pub fn x(&self, k: usize) -> f32 {
        assert!(k < self.count);
        self.x[k]
    }

    pub fn width(&self, k: usize) -> f32 {
        assert!(k < self.count);
        self.width[k]
    }

    /// The x of the divider between column `k` and `k + 1`.
    pub fn divider_x(&self, k: usize) -> f32 {
        assert!(k + 1 < self.count);
        match self.aside {
            // The gap between the float and its channel, whichever is first.
            Some(true) => self.x[1] + self.width[1] + GAP / 2.,
            Some(false) => self.x[0] + self.width[0] + GAP / 2.,
            None => self.x[k + 1] - GAP / 2.,
        }
    }
}

/// The columns' places for shares `percent` (summing to 100) across `total`
/// pixels, or `None` when any would be under [`MIN_COLUMN`]: the region then
/// stacks. The gaps come off the total first, so shares are of what is left.
pub fn split(total: f32, percent: &[u8]) -> Option<Split> {
    let count = percent.len();
    assert!((2..=MAX_COLUMNS).contains(&count), "{count} columns");
    if !total.is_finite() {
        return None;
    }
    let usable = total - GAP * (count - 1) as f32;
    let mut out = Split {
        count,
        x: [0.; MAX_COLUMNS],
        width: [0.; MAX_COLUMNS],
        aside: None,
        total,
    };
    let mut x = 0.;
    for (k, &share) in percent.iter().enumerate() {
        let width = usable * f32::from(share) / 100.;
        if width < MIN_COLUMN {
            return None;
        }
        (out.x[k], out.width[k]) = (x, width);
        x += width + GAP;
    }
    Some(out)
}

/// A float narrower than this is not worth wrapping text around.
pub const MIN_FLOAT: f32 = 120.;

/// The places of an aside's float (`x(0)`, `width(0)`) and of the text channel
/// beside it (`x(1)`, `width(1)`) across a line `total` pixels wide, or `None`
/// when either is too narrow: the region then stacks.
pub fn aside_split(total: f32, right: bool, share: u8) -> Option<Split> {
    if !total.is_finite() {
        return None;
    }
    let usable = total - GAP;
    let float = usable * f32::from(share) / 100.;
    let channel = usable - float;
    if float < MIN_FLOAT || channel < MIN_COLUMN {
        return None;
    }
    let mut out = Split {
        count: 2,
        x: [0.; MAX_COLUMNS],
        width: [0.; MAX_COLUMNS],
        aside: Some(right),
        total,
    };
    let (float_x, channel_x) = if right {
        (channel + GAP, 0.)
    } else {
        (0., float + GAP)
    };
    (out.x[0], out.width[0]) = (float_x, float);
    (out.x[1], out.width[1]) = (channel_x, channel);
    Some(out)
}

/// Whether `line` is the open or close marker of one of `regions` (sorted).
pub fn is_frame_line(regions: &[Columns], line: usize) -> bool {
    let i = regions.partition_point(|r| r.end_line <= line);
    regions.get(i).is_some_and(|r| r.is_frame(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_text(text: &str) -> Vec<Columns> {
        scan(text.lines().map(|l| (l, false)))
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
        // The separator is inside a fence: no second column.
        let lines = text.lines().enumerate().map(|(i, l)| (l, i == 4));
        assert!(scan(lines).is_empty());
    }

    #[test]
    fn a_document_end_counts_as_blank() {
        assert_eq!(scan_text("::: columns\n\na\n\n---\n\nb\n\n:::").len(), 1);
        assert!(scan_text("").is_empty());
    }
    #[test]
    fn frame_lines_are_the_open_and_close_markers_only() {
        let regions = scan_text(&format!("{TWO}\nx\n\n{TWO}"));
        assert_eq!(regions.len(), 2);
        let framed: Vec<usize> = (0..25).filter(|&l| is_frame_line(&regions, l)).collect();
        // Each region's open (0, 9+...) and close lines; never a separator.
        let expected: Vec<usize> = regions
            .iter()
            .flat_map(|r| [r.start_line, r.close_line()])
            .collect();
        assert_eq!(framed, expected);
        assert!(!is_frame_line(&regions, regions[0].separators()[0]));
    }
    #[test]
    fn split_shares_what_the_gaps_leave() {
        let s = split(1000., &[60, 40]).unwrap();
        // 1000 - 24 = 976 usable.
        assert!((s.width(0) - 585.6).abs() < 0.01 && (s.width(1) - 390.4).abs() < 0.01);
        assert_eq!((s.x(0), s.count), (0., 2));
        assert!((s.x(1) - (585.6 + GAP)).abs() < 0.01);
        assert!((s.x(1) + s.width(1) - 1000.).abs() < 0.01, "fills the line");
        assert!((s.divider_x(0) - (585.6 + GAP / 2.)).abs() < 0.01);
    }

    #[test]
    fn split_of_four_fills_the_line() {
        let s = split(1000., &[25, 25, 25, 25]).unwrap();
        let end = s.x(3) + s.width(3);
        assert!((end - 1000.).abs() < 0.01, "{end}");
    }

    #[test]
    fn a_column_under_the_minimum_stacks_the_region() {
        // 20%: (total - 24) * 0.2 >= 160 needs total >= 824.
        assert!(split(830., &[20, 80]).is_some());
        assert!(split(820., &[20, 80]).is_none());
        assert!(split(100., &[50, 50]).is_none());
        assert!(split(f32::NAN, &[50, 50]).is_none());
        assert!(split(f32::INFINITY, &[50, 50]).is_none());
    }
    #[test]
    fn column_of_names_content_lines_only() {
        let r = scan_text(TWO)[0];
        assert_eq!(r.column_of(2), Some(0));
        assert_eq!(r.column_of(6), Some(1));
        for line in [0, 1, 3, 4, 5, 7, 8] {
            assert_eq!(r.column_of(line), None, "line {line}");
        }
    }

    #[test]
    fn an_aside_opens_with_a_side_and_a_share_in_any_order() {
        let aside = |open: &str| {
            let text = format!("{open}\n\nblock\n\n---\n\ntext\n\n:::\n");
            scan_text(&text)
                .first()
                .map(|r| (r.kind, r.percent[0], r.percent[1]))
        };
        let right = |share| Some((Kind::Aside { right: true }, share, 100 - share));
        let left = |share| Some((Kind::Aside { right: false }, share, 100 - share));
        assert_eq!(aside("::: aside"), right(40));
        assert_eq!(aside("::: aside right 30%"), right(30));
        assert_eq!(aside("::: aside left 25"), left(25));
        assert_eq!(aside("::: aside 60% left"), left(60));
        for bad in [
            "::: aside middle",
            "::: aside 5%",
            "::: aside 95",
            "::: aside left left",
            "::: asidex",
            "::: aside 40% 50%",
        ] {
            assert_eq!(aside(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_aside_needs_exactly_two_parts() {
        let three = "::: aside\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::\n";
        assert!(scan_text(three).is_empty());
        let one = "::: aside\n\na\n\n:::\n";
        assert!(scan_text(one).is_empty());
    }

    #[test]
    fn an_aside_split_puts_the_float_on_its_side() {
        let total = 1000.;
        let right = aside_split(total, true, 40).unwrap();
        let left = aside_split(total, false, 40).unwrap();
        let usable = total - GAP;
        assert_eq!(right.width(0), usable * 0.4);
        assert_eq!(right.x(1), 0.);
        assert_eq!(right.x(0), right.width(1) + GAP);
        assert_eq!(left.x(0), 0.);
        assert_eq!(left.x(1), left.width(0) + GAP);
        // The grip sits in the middle of the gap.
        assert_eq!(right.divider_x(0), right.width(1) + GAP / 2.);
        assert_eq!(left.divider_x(0), left.width(0) + GAP / 2.);
        // A channel under the minimum stacks the region.
        assert!(aside_split(300., true, 40).is_none());
        assert!(
            aside_split(1000., true, 10).is_none(),
            "a 98px float is too small"
        );
    }
}

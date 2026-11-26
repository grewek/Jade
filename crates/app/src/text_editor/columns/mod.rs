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

use std::ops::Range;

pub(in crate::text_editor) mod handles;
pub(in crate::text_editor) mod ops;
mod split;

pub use split::*;

pub const MAX_COLUMNS: usize = 4;

const OPEN: &str = "::: columns";
const OPEN_ASIDE: &str = "::: aside";
const OPEN_FLOAT: &str = "::: float";
/// A line of its own in a float region's text: the text below it starts under
/// every float above it.
const CLEAR: &str = "::: clear";
/// The clear markers one region keeps.
pub const MAX_CLEARS: usize = 3;
/// An aside's float share, in percent of the line (without the gap).
pub const ASIDE_MIN: u8 = 10;
pub const ASIDE_MAX: u8 = 90;
pub const ASIDE_DEFAULT: u8 = 40;
const CLOSE: &str = ":::";
const SEPARATOR: &str = "---";

/// What a region lays out as: equal-height columns, or blocks floated to the
/// sides with the last part, the text, flowing around them (`::: aside right
/// 40%`). An aside has 1 to [`MAX_FLOATS`] floats, each a part of its own,
/// then the text as the last part; the first float's side and share are on the
/// open marker, each further float's on the `--- left 30%` separator that
/// starts it, and the plain `---` starts the text. Its `percent` holds the
/// floats' shares, then 0 for the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Columns,
    Aside,
}

/// The floats an aside can have.
pub const MAX_FLOATS: usize = MAX_COLUMNS - 1;

/// One float's side and share (percent of the line, without the gaps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloatSpec {
    pub right: bool,
    pub share: u8,
}

const NO_FLOAT: FloatSpec = FloatSpec {
    right: true,
    share: ASIDE_DEFAULT,
};

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
    /// An aside's floats; the first `count - 1` are used.
    floats: [FloatSpec; MAX_FLOATS],
    /// A float block (`::: float`, the float, `:::`) with no close marker for
    /// its text: the text is the lines after the `:::` up to `end_line`, which
    /// [`Self::cap_tail`] sets. `end_line` is then no marker line.
    pub tail: bool,
    /// Where a tail's text would end with no cap: the next region's open marker
    /// or the document's end. Unused (0) otherwise.
    tail_bound: usize,
    /// A tail's further floats' open marker lines (a float block close enough
    /// after the one before joins its region); `opens[k]` is float `k`'s, the
    /// first float's being `start_line`. Their `:::` is `separators[k]`.
    opens: [usize; MAX_FLOATS],
    /// A tail's `::: clear` lines; the first `clear_count` are used.
    clears: [usize; MAX_CLEARS],
    clear_count: usize,
}

/// A float's text reaches this many lines past it, plus [`TAIL_PER_LINE`] for
/// each line of the float, so that it covers the rows the float takes (a line
/// has at least one row). The unit ends at the next blank line after that, so
/// no block is cut. A float taller than its text leaves a gap below the text.
pub const TAIL_BASE: usize = 16;
pub const TAIL_PER_LINE: usize = 4;

impl Columns {
    /// Whether this is floated blocks with text around them.
    pub fn is_aside(&self) -> bool {
        self.kind == Kind::Aside
    }

    /// An aside's floats, in order, with their shares as written.
    pub fn floats(&self) -> &[FloatSpec] {
        if self.is_aside() {
            &self.floats[..self.count - 1]
        } else {
            &[]
        }
    }

    /// This region with `shares` instead of its own (one per column; for an
    /// aside, per float and then the text's unused one).
    pub fn with_shares(mut self, shares: &[u8]) -> Self {
        assert_eq!(shares.len(), self.count);
        self.percent[..self.count].copy_from_slice(shares);
        if self.is_aside() {
            for (f, &share) in self.floats.iter_mut().zip(shares) {
                f.share = share;
            }
        }
        self
    }

    /// The separator lines between the columns, in order.
    pub fn separators(&self) -> &[usize] {
        &self.separators[..self.count - 1]
    }

    /// The close marker's line; for a tail region the `:::` that ends its last
    /// float, the region itself having none.
    pub fn close_line(&self) -> usize {
        if self.tail {
            self.separators[self.count - 2]
        } else {
            self.end_line - 1
        }
    }

    /// The line of float `k`'s open marker (a tail's).
    pub fn open_of(&self, k: usize) -> usize {
        if k == 0 {
            self.start_line
        } else {
            self.opens[k]
        }
    }

    /// The lines the floats of a tail take, markers included.
    fn float_lines(&self) -> usize {
        (0..self.count - 1)
            .map(|k| self.separators[k] - self.open_of(k))
            .sum()
    }

    /// How far past a tail's last float another float block may open and still
    /// join its region: the reach of its text.
    fn reach(&self) -> usize {
        TAIL_BASE + TAIL_PER_LINE * self.float_lines()
    }

    /// A tail's `::: clear` lines, in order: those inside its text, the cap
    /// possibly leaving later ones out.
    pub fn clears(&self) -> &[usize] {
        let all = &self.clears[..self.clear_count];
        let inside = all.iter().take_while(|&&c| c + 1 < self.end_line).count();
        &all[..inside]
    }

    /// Whether `line` is in a tail's further float block (open marker to the
    /// padding line after its `:::`) or a clear marker or its padding line:
    /// lines the text flows past.
    fn in_float_block(&self, line: usize) -> bool {
        self.tail
            && ((1..self.count - 1)
                .any(|k| (self.opens[k]..=self.separators[k] + 1).contains(&line))
                || self.clears().iter().any(|&c| line == c || line == c + 1))
    }

    /// Sets a tail region's `end_line`: its text takes the lines after the float
    /// block up to the first line `ends_text` accepts at or after the budget,
    /// else up to the bound. A region that is no tail is left alone.
    pub fn cap_tail(&mut self, ends_text: impl Fn(usize) -> bool) {
        if !self.tail {
            return;
        }
        let want = self.close_line() + 1 + self.reach();
        self.end_line = (want..self.tail_bound)
            .find(|&line| ends_text(line))
            .unwrap_or(self.tail_bound);
    }

    /// The line after which float `k` is put down: the line its block is
    /// written at in a tail, the top of an aside for every float.
    pub fn float_anchor(&self, k: usize) -> usize {
        if self.tail && k > 0 { self.opens[k] } else { 0 }
    }

    /// The lines of the text part: for a tail, its lines but those of the
    /// floats that open among them.
    pub fn text_lines(&self) -> impl Iterator<Item = usize> + '_ {
        self.column_lines(self.count - 1)
            .filter(|&line| !self.in_float_block(line))
    }

    /// The column whose content lines include `line`; `None` for a marker or
    /// a padding blank line.
    pub fn column_of(&self, line: usize) -> Option<usize> {
        (0..self.count).find(|&k| {
            self.column_lines(k).contains(&line)
                && (k + 1 < self.count || !self.in_float_block(line))
        })
    }

    /// Whether `line` is the open or the close marker, the two that render as
    /// nothing (a separator renders as the usual rule). A tail's floats'
    /// markers all are.
    pub fn is_frame(&self, line: usize) -> bool {
        if self.tail {
            return self.is_marker(line);
        }
        line == self.start_line || line == self.close_line()
    }

    /// Whether `line` is one of the region's three kinds of marker line.
    pub fn is_marker(&self, line: usize) -> bool {
        line == self.start_line
            || line == self.close_line()
            || self.separators().contains(&line)
            || (self.tail
                && (self.opens[1..self.count - 1].contains(&line) || self.clears().contains(&line)))
    }

    /// The lines of column `k`, without the blank line that pads each marker.
    /// A column with nothing but padding (a hand-typed one has a single blank
    /// line) is all of it, so the caret still has a line there; typing then
    /// adds the padding (`Editor::insert_in_bare_column`). A tail's
    /// text part is the whole stretch after its first float; the lines of its
    /// later floats are in it too, see [`Self::text_lines`].
    pub fn column_lines(&self, k: usize) -> Range<usize> {
        assert!(k < self.count, "column {k} of {}", self.count);
        if self.tail && k + 1 == self.count {
            let start = self.separators[0] + 2;
            return start..self.end_line.max(start);
        }
        let (before, after) = if self.tail {
            (self.open_of(k), self.separators[k])
        } else {
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
            (first, next)
        };
        if before + 2 < after - 1 {
            before + 2..after - 1
        } else {
            before + 1..after
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// `::: columns`, with its explicit ratio if one was written.
    Open(Option<Ratio>),
    /// `::: aside [left|right] [N%]`.
    OpenAside(FloatSpec),
    /// `::: float [left|right] [N%]`: a float whose text is what follows its
    /// closing `:::`.
    OpenFloat(FloatSpec),
    /// `::: clear`.
    Clear,
    Separator,
    /// `--- [left|right] [N%]`: starts a further float of an aside.
    FloatSeparator(FloatSpec),
    Close,
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
    aside: bool,
    /// A float block: closed by its `:::`, its text is the lines after.
    float: bool,
    floats: [FloatSpec; MAX_FLOATS],
    /// The floats so far (an aside's open marker is the first).
    float_count: usize,
    /// Whether the plain `---` that starts an aside's text was seen.
    text_seen: bool,
    separators: [usize; MAX_COLUMNS - 1],
    seen: usize,
}

impl Pending {
    /// The finished region, or `None` when the ratio and the column count
    /// disagree.
    fn close(self, line: usize) -> Option<Columns> {
        let count = self.seen + 1;
        if self.aside {
            // Floats, then the text after the one plain separator.
            if !self.text_seen || self.float_count != count - 1 {
                return None;
            }
            let mut percent = [0; MAX_COLUMNS];
            for (p, f) in percent.iter_mut().zip(&self.floats[..self.float_count]) {
                *p = f.share;
            }
            return Some(Columns {
                kind: Kind::Aside,
                start_line: self.start,
                end_line: line + 1,
                count,
                percent,
                separators: self.separators,
                floats: self.floats,
                tail: false,
                tail_bound: 0,
                opens: [0; MAX_FLOATS],
                clears: [0; MAX_CLEARS],
                clear_count: 0,
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
            floats: [NO_FLOAT; MAX_FLOATS],
            tail: false,
            tail_bound: 0,
            opens: [0; MAX_FLOATS],
            clears: [0; MAX_CLEARS],
            clear_count: 0,
        })
    }

    /// The region a float block makes; its text's end is set once every
    /// marker is known (see [`derive`]).
    fn close_float(self, line: usize) -> Columns {
        let mut separators = [0; MAX_COLUMNS - 1];
        separators[0] = line;
        let mut percent = [0; MAX_COLUMNS];
        percent[0] = self.floats[0].share;
        Columns {
            kind: Kind::Aside,
            start_line: self.start,
            end_line: line + 1,
            count: 2,
            percent,
            separators,
            floats: self.floats,
            tail: true,
            tail_bound: 0,
            opens: [0; MAX_FLOATS],
            clears: [0; MAX_CLEARS],
            clear_count: 0,
        }
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
    if let Some(rest) = line.strip_prefix(SEPARATOR).filter(|r| r.starts_with(' ')) {
        return parse_aside(rest).map(Marker::FloatSeparator);
    }
    if line == CLOSE {
        return Some(Marker::Close);
    }
    if line == CLEAR {
        return Some(Marker::Clear);
    }
    if let Some(rest) = line.strip_prefix(OPEN_ASIDE) {
        return parse_aside(rest).map(Marker::OpenAside);
    }
    if let Some(rest) = line.strip_prefix(OPEN_FLOAT) {
        return parse_aside(rest).map(Marker::OpenFloat);
    }
    let rest = line.strip_prefix(OPEN)?;
    if rest.is_empty() {
        return Some(Marker::Open(None));
    }
    parse_ratio(rest.strip_prefix(' ')?.trim_start()).map(|r| Marker::Open(Some(r)))
}

/// What follows `::: aside`: nothing, or `left`/`right` and a share (`40` or
/// `40%`) in either order, each at most once. Right and 40% by default.
fn parse_aside(rest: &str) -> Option<FloatSpec> {
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
    Some(FloatSpec {
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

/// Whether `marker` opens a region.
fn opens(marker: Marker) -> bool {
    matches!(
        marker,
        Marker::Open(_) | Marker::OpenAside(_) | Marker::OpenFloat(_)
    )
}

/// The regions the markers form, in order (see the module comment for the
/// rules). `markers` must be sorted by line; `line_count` is the document's,
/// where the text of a last float ends.
pub fn derive(markers: &[Placed], line_count: usize) -> Vec<Columns> {
    let mut regions = Vec::new();
    let mut pending: Option<Pending> = None;
    for &Placed { line, marker } in markers {
        match marker {
            Marker::Open(ratio) => {
                pending = Some(Pending {
                    start: line,
                    ratio,
                    aside: false,
                    float: false,
                    floats: [NO_FLOAT; MAX_FLOATS],
                    float_count: 0,
                    text_seen: false,
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            Marker::OpenAside(spec) => {
                let mut floats = [NO_FLOAT; MAX_FLOATS];
                floats[0] = spec;
                pending = Some(Pending {
                    start: line,
                    ratio: None,
                    aside: true,
                    float: false,
                    floats,
                    float_count: 1,
                    text_seen: false,
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            Marker::OpenFloat(spec) => {
                let mut floats = [NO_FLOAT; MAX_FLOATS];
                floats[0] = spec;
                pending = Some(Pending {
                    start: line,
                    ratio: None,
                    aside: false,
                    float: true,
                    floats,
                    float_count: 1,
                    text_seen: false,
                    separators: [0; MAX_COLUMNS - 1],
                    seen: 0,
                });
            }
            // A rule inside a float block is part of the float.
            Marker::Separator => {
                if let Some(p) = pending.as_mut().filter(|p| !p.float) {
                    // A second plain separator in an aside: its text is over.
                    if p.seen == MAX_COLUMNS - 1 || (p.aside && p.text_seen) {
                        pending = None;
                    } else {
                        p.separators[p.seen] = line;
                        p.seen += 1;
                        p.text_seen = p.aside;
                    }
                }
            }
            // Only an aside before its text takes this; elsewhere it is text.
            Marker::FloatSeparator(spec) => {
                if let Some(p) = pending.as_mut().filter(|p| p.aside) {
                    if p.float_count == MAX_FLOATS || p.text_seen {
                        pending = None;
                    } else {
                        p.separators[p.seen] = line;
                        p.seen += 1;
                        p.floats[p.float_count] = spec;
                        p.float_count += 1;
                    }
                }
            }
            // Handled once the regions are known.
            Marker::Clear => {}
            Marker::Close => {
                if pending.as_ref().is_some_and(|p| p.float)
                    && let Some(p) = pending.take()
                {
                    let spec = p.floats[0];
                    // A float block opening within reach of the last one's
                    // text joins its region: both float, the text flows
                    // between and around them.
                    let joined = regions.last_mut().filter(|last: &&mut Columns| {
                        last.tail
                            && last.count - 1 < MAX_FLOATS
                            && p.start - last.close_line() <= last.reach()
                    });
                    match joined {
                        Some(last) => {
                            let k = last.count - 1;
                            last.floats[k] = spec;
                            last.separators[k] = line;
                            last.opens[k] = p.start;
                            last.percent[k] = spec.share;
                            last.percent[k + 1] = 0;
                            last.count += 1;
                        }
                        None => regions.push(p.close_float(line)),
                    }
                // A region needs a separator: one column is not columns.
                } else if let Some(p) = pending.take().filter(|p| p.seen >= 1) {
                    regions.extend(p.close(line));
                }
            }
        }
    }
    // A float's text runs to the next region's open marker.
    for region in regions.iter_mut().filter(|r| r.tail) {
        let after = markers.partition_point(|m| m.line <= region.close_line());
        region.tail_bound = markers[after..]
            .iter()
            .find(|m| opens(m.marker))
            .map_or(line_count, |m| m.line);
        region.end_line = region.tail_bound;
    }
    // A clear marker belongs to the float region whose text it is in.
    for &Placed { line, marker } in markers {
        if marker != Marker::Clear {
            continue;
        }
        let i = regions.partition_point(|r| r.end_line <= line);
        if let Some(r) = regions
            .get_mut(i)
            .filter(|r| r.tail && line > r.close_line() && r.clear_count < MAX_CLEARS)
        {
            r.clears[r.clear_count] = line;
            r.clear_count += 1;
        }
    }
    regions
}

/// Every region of a document, `lines` being as for [`place_markers`]. The
/// oracle the cache's incremental markers are tested against.
#[cfg(test)]
pub fn scan<'a>(lines: impl Iterator<Item = (&'a str, bool)>) -> Vec<Columns> {
    let lines: Vec<_> = lines.collect();
    let count = lines.len();
    let mut regions = derive(&place_markers(0, lines.iter().copied(), true, true), count);
    for region in &mut regions {
        region.cap_tail(|line| {
            lines
                .get(line)
                .is_some_and(|&(text, code)| !code && is_blank(text))
        });
    }
    regions
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
    fn a_column_of_one_blank_line_has_that_line() {
        let r = scan_text("::: columns\n\n---\n\n:::")[0];
        assert_eq!(r.column_lines(0), 1..2);
        assert_eq!(r.column_lines(1), 3..4);
        // A second blank line (Enter there) is content too, until there is
        // room for a padding line on each side of it.
        let r = scan_text("::: columns\n\n\n---\n\n\n\n:::")[0];
        assert_eq!(r.column_lines(0), 1..3);
        assert_eq!(r.column_lines(1), 5..6);
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

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn one(right: bool, share: u8) -> [FloatSpec; 1] {
        [FloatSpec { right, share }]
    }

    #[test]
    fn an_aside_opens_with_a_side_and_a_share_in_any_order() {
        let aside = |open: &str| {
            let text = format!("{open}\n\nblock\n\n---\n\ntext\n\n:::\n");
            scan_text(&text)
                .first()
                .map(|r| (r.kind, r.floats().to_vec()))
        };
        let expect = |right, share| Some((Kind::Aside, one(right, share).to_vec()));
        assert_eq!(aside("::: aside"), expect(true, 40));
        assert_eq!(aside("::: aside right 30%"), expect(true, 30));
        assert_eq!(aside("::: aside left 25"), expect(false, 25));
        assert_eq!(aside("::: aside 60% left"), expect(false, 60));
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
    fn an_aside_has_floats_then_one_text_part() {
        // Two floats, then the text: each further float starts at a `--- side N%`.
        let two = "::: aside left 30%\n\na\n\n--- right 25%\n\nb\n\n---\n\ntext\n\n:::\n";
        let r = scan_text(two)[0];
        assert_eq!(r.count, 3);
        assert_eq!(
            r.floats(),
            [
                FloatSpec {
                    right: false,
                    share: 30
                },
                FloatSpec {
                    right: true,
                    share: 25
                }
            ]
        );
        assert_eq!(r.percent[..3], [30, 25, 0]);
        assert_eq!(r.column_of(2), Some(0));
        assert_eq!(r.column_of(6), Some(1));
        assert_eq!(r.column_of(10), Some(2));
        // Three floats is the most; a fourth is not a region.
        let three = "::: aside\n\na\n\n--- left\n\nb\n\n--- right\n\nc\n\n---\n\ntext\n\n:::\n";
        assert_eq!(scan_text(three)[0].floats().len(), 3);
        let four = "::: aside\n\na\n\n--- left\n\nb\n\n--- right\n\nc\n\n--- left\n\nd\n\n---\n\ntext\n\n:::\n";
        assert!(scan_text(four).is_empty());
        // The text part is last and plain, and needs to be there.
        for bad in [
            "::: aside\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::\n",
            "::: aside\n\na\n\n---\n\nb\n\n--- left\n\nc\n\n:::\n",
            "::: aside\n\na\n\n--- left\n\nb\n\n:::\n",
            "::: aside\n\na\n\n:::\n",
        ] {
            assert!(scan_text(bad).is_empty(), "{bad}");
        }
        // A spec'd separator is only a separator in an aside.
        let columns = "::: columns\n\na\n\n--- left\n\nb\n\n:::\n";
        assert!(scan_text(columns).is_empty());
    }

    #[test]
    fn an_aside_split_puts_the_float_on_its_side() {
        let total = 1000.;
        let right = aside_split(total, &one(true, 40)).unwrap();
        let left = aside_split(total, &one(false, 40)).unwrap();
        let usable = total - GAP;
        assert!(near(right.width(0), usable * 0.4));
        assert_eq!(right.x(1), 0.);
        assert!(near(right.x(0), right.width(1) + GAP));
        assert_eq!(left.x(0), 0.);
        assert!(near(left.x(1), left.width(0) + GAP));
        // The grip sits in the middle of the gap.
        assert!(near(right.divider_x(0), right.width(1) + GAP / 2.));
        assert!(near(left.divider_x(0), left.width(0) + GAP / 2.));
        // A channel under the minimum stacks the region.
        assert!(aside_split(300., &one(true, 40)).is_none());
        // A float asking for more than leaves the text its width gets narrower.
        let narrow = aside_split(800., &one(true, 50)).unwrap();
        assert!(near(narrow.width(1), MIN_CHANNEL));
        assert!(near(narrow.width(0), 800. - GAP - MIN_CHANNEL));
        // And when that leaves under the float's minimum, the region stacks.
        assert!(aside_split(700., &one(true, 40)).is_none());
        assert!(
            aside_split(1000., &one(true, 10)).is_none(),
            "a 98px float is too small"
        );
    }

    #[test]
    fn floats_on_both_sides_leave_the_text_between_them() {
        let floats = [
            FloatSpec {
                right: false,
                share: 20,
            },
            FloatSpec {
                right: true,
                share: 20,
            },
            FloatSpec {
                right: false,
                share: 10,
            },
        ];
        let total = 1400.;
        let split = aside_split(total, &floats).unwrap();
        let usable = total - 2. * GAP;
        // The two left floats are as wide as the wider of them.
        assert!(near(split.width(0), usable * 0.2));
        assert!(near(split.width(2), usable * 0.2));
        assert_eq!(split.x(0), 0.);
        assert!(near(split.x(1), total - usable * 0.2));
        // The text (part 3) is what is left between them.
        assert!(near(split.x(3), usable * 0.2 + GAP));
        assert!(near(split.width(3), total - 2. * (usable * 0.2 + GAP)));
        // One grip per side, the left one first.
        assert_eq!(split.divider_count(), 2);
        assert!(!split.divider_is_right(0) && split.divider_is_right(1));
        assert!(near(split.divider_x(0), usable * 0.2 + GAP / 2.));
        assert!(near(split.divider_x(1), split.x(1) - GAP / 2.));
        // Both sides asking for a lot share what the text leaves.
        let wide = [
            FloatSpec {
                right: false,
                share: 45,
            },
            FloatSpec {
                right: true,
                share: 45,
            },
        ];
        let split = aside_split(1400., &wide).unwrap();
        assert!(near(split.width(2), MIN_CHANNEL));
        assert!((split.width(0) - split.width(1)).abs() < 0.01);
        // Too narrow for both: the region stacks.
        assert!(aside_split(800., &wide).is_none());
    }

    #[test]
    fn unchecked_geometry_matches_the_checked_one_where_both_exist() {
        let total = 1000.;
        let percent = [20, 50, 30];
        let columns = scan_text("::: columns 20/50/30\n\na\n\n---\n\nb\n\n---\n\nc\n\n:::\n")[0];
        let split = split(total, &percent).unwrap();
        for k in 0..2 {
            assert!(
                (divider_x_unchecked(total, &columns, &percent, k) - split.divider_x(k)).abs()
                    < 0.01
            );
        }
        for k in 0..3 {
            let (left, usable) = column_left_unchecked(total, &percent, k);
            assert!((left - split.x(k)).abs() < 0.01);
            assert!((usable - (total - 2. * GAP)).abs() < 0.01);
        }
        for right in [true, false] {
            let aside = aside_split(total, &one(right, 40)).unwrap();
            let region = scan_text(&format!(
                "::: aside {} 40%\n\nf\n\n---\n\nt\n\n:::\n",
                if right { "right" } else { "left" }
            ))[0];
            assert!(
                (divider_x_unchecked(total, &region, &[40, 0], 0) - aside.divider_x(0)).abs()
                    < 0.01
            );
        }
        // Where the checked one gives up, the unchecked one still has a place.
        assert!(split_is_none(total, &[95, 5]));
        assert!(divider_x_unchecked(total, &columns, &[95, 5], 0) > 900.);
    }

    fn split_is_none(total: f32, percent: &[u8]) -> bool {
        split(total, percent).is_none()
    }

    #[test]
    fn a_stacked_region_is_restored_to_equal_shares_only_when_they_fit() {
        let text = "::: columns 90/10\n\na\n\n---\n\nb\n\n:::\n";
        let region = scan_text(text)[0];
        assert_eq!(restored(&region, 1000.), Some(vec![50, 50]));
        assert_eq!(restored(&region, 300.), None, "too narrow even when equal");
        let aside = scan_text("::: aside left 90%\n\na\n\n---\n\nb\n\n:::\n")[0];
        assert_eq!(restored(&aside, 1000.), Some(vec![ASIDE_DEFAULT, 0]));
    }

    /// A float block: the float, its closing `:::`, and the lines after it as
    /// the text, up to the next region's open marker.
    #[test]
    fn a_float_block_takes_the_lines_after_it_as_its_text() {
        let text = "::: float left 30%\n\nfloat\n\n:::\n\ntext one\n\ntext two\n\n::: columns\n\na\n\n---\n\nb\n\n:::\n";
        let regions = scan_text(text);
        assert_eq!(regions.len(), 2);
        let r = regions[0];
        assert!(r.tail && r.is_aside());
        assert_eq!((r.start_line, r.end_line, r.count), (0, 10, 2));
        assert_eq!(
            r.floats(),
            &[FloatSpec {
                right: false,
                share: 30
            }]
        );
        assert_eq!(r.column_lines(0), 2..3);
        assert_eq!(r.column_lines(1), 6..10);
        assert!(r.is_frame(0) && r.is_frame(4));
        assert!(!r.is_frame(9));
        assert_eq!(regions[1].start_line, 10);
    }

    #[test]
    fn a_float_blocks_text_runs_to_the_end_of_the_document() {
        let regions = scan_text("::: float\n\nf\n\n:::\n\nafter");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].end_line, 7);
        assert_eq!(regions[0].column_lines(1), 6..7);
    }

    /// The text is capped at the first blank line past the budget, so a long
    /// document is not one region and no paragraph is cut.
    #[test]
    fn a_float_blocks_text_is_capped_at_a_blank_line() {
        let mut text = String::from("::: float\n\nf\n\n:::\n\n");
        for i in 0..200 {
            text.push_str(&format!("line {i}\n\n"));
        }
        let r = scan_text(&text)[0];
        let want = 4 + 1 + TAIL_BASE + TAIL_PER_LINE * 4;
        assert!(
            r.end_line >= want && r.end_line <= want + 2,
            "{}",
            r.end_line
        );
        // The line the text ends before is blank: a paragraph is whole.
        assert!(text.lines().nth(r.end_line).unwrap().is_empty());
    }

    #[test]
    fn a_float_block_is_not_one_without_its_close_or_when_the_marker_is_bad() {
        assert!(scan_text("::: float\n\nf\n").is_empty());
        assert!(scan_text("::: float middle\n\nf\n\n:::\n").is_empty());
        // A rule inside is part of the float.
        let r = scan_text("::: float\n\nf\n\n---\n\ng\n\n:::\n\ntext")[0];
        assert_eq!(r.column_lines(0), 2..7);
        assert_eq!(r.column_lines(1), 10..11);
    }

    /// A float block opening within reach of the last one's text joins its
    /// region; the text part holds the lines between and after the floats.
    #[test]
    fn a_float_block_within_reach_joins_the_region() {
        let text =
            "::: float left 20%\n\nA\n\n:::\n\none\n\n::: float right 25%\n\nB\n\n:::\n\ntwo\n";
        let regions = scan_text(text);
        assert_eq!(regions.len(), 1);
        let r = regions[0];
        assert_eq!((r.count, r.start_line, r.end_line), (3, 0, 15));
        assert_eq!(
            r.floats(),
            &[
                FloatSpec {
                    right: false,
                    share: 20
                },
                FloatSpec {
                    right: true,
                    share: 25
                },
            ]
        );
        assert_eq!(r.column_lines(0), 2..3);
        assert_eq!(r.column_lines(1), 10..11);
        // The text is `one` and `two` (and the blank line before the second
        // float), not the second float's own lines.
        assert_eq!(r.text_lines().collect::<Vec<_>>(), [6, 7, 14]);
        assert_eq!(r.column_of(10), Some(1));
        assert_eq!(r.column_of(6), Some(2));
        assert_eq!(r.column_of(9), None, "the padding after the open marker");
        assert!([0, 4, 8, 12].iter().all(|&l| r.is_frame(l)));
    }

    #[test]
    fn a_float_block_beyond_reach_or_past_three_floats_starts_a_region() {
        let mut far = String::from("::: float\n\nA\n\n:::\n\n");
        for _ in 0..40 {
            far.push_str("filler\n\n");
        }
        far.push_str("::: float\n\nB\n\n:::\n\ntext\n");
        assert_eq!(scan_text(&far).len(), 2);

        let four = "::: float\n\n1\n\n:::\n\n::: float\n\n2\n\n:::\n\n::: float\n\n3\n\n:::\n\n::: float\n\n4\n\n:::\n\nt\n";
        let regions = scan_text(four);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].count, 4, "three floats and the text");
        assert_eq!(regions[1].count, 2);
        // A column region between them keeps them apart.
        let apart = "::: float\n\nA\n\n:::\n\n::: columns\n\na\n\n---\n\nb\n\n:::\n\n::: float\n\nB\n\n:::\n\nt\n";
        assert_eq!(scan_text(apart).len(), 3);
    }

    /// `::: clear` in a float region's text is a hidden marker: the text below
    /// it comes after the floats above.
    #[test]
    fn a_clear_marker_belongs_to_the_float_text_it_is_in() {
        let text = "::: float left\n\nA\n\n:::\n\none\n\n::: clear\n\ntwo\n";
        let r = scan_text(text)[0];
        assert_eq!(r.clears(), &[8]);
        assert!(r.is_marker(8) && r.is_frame(8));
        assert_eq!(r.text_lines().collect::<Vec<_>>(), [6, 7, 10]);
        assert_eq!(r.column_of(8), None);
        assert_eq!(r.column_of(9), None, "its padding line");
        // Outside any float's text it is plain text.
        assert!(scan_text("one\n\n::: clear\n\ntwo\n").is_empty());
        let before = scan_text("::: clear\n\n::: float\n\nA\n\n:::\n\nt\n");
        assert!(before[0].clears().is_empty());
    }
}

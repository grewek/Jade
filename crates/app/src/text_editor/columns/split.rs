//! Pixel geometry of a region: where each column or float sits across a line
//! of a given width, and the ratio restored when a stacked region widens
//! again. Pure arithmetic on the region's shares (parsing is in `mod.rs`).

use super::*;

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
    /// Whether this is an aside: parts `0..count - 1` are its floats (each at
    /// its side's width) and the last is the text channel between them (the
    /// text below the floats uses the whole line, see `total`).
    pub aside: bool,
    /// For an aside, whether float `k` is on the right.
    float_right: [bool; MAX_FLOATS],
    /// The x of each divider: between columns, or the inner edge of the left
    /// and then the right side of floats.
    dividers: [f32; MAX_COLUMNS - 1],
    divider_count: usize,
    /// For an aside, whether divider `k` is on the right side.
    divider_right: [bool; 2],
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

    /// How many dividers have a grip: between columns, or one per side that
    /// has floats.
    pub fn divider_count(&self) -> usize {
        self.divider_count
    }

    /// The x of divider `k` (the one between column `k` and `k + 1`; an
    /// aside's are its left side's and then its right side's).
    pub fn divider_x(&self, k: usize) -> f32 {
        assert!(k < self.divider_count);
        self.dividers[k]
    }

    /// For an aside, whether divider `k` is the right side's.
    pub fn divider_is_right(&self, k: usize) -> bool {
        assert!(self.aside && k < self.divider_count);
        self.divider_right[k]
    }

    /// For an aside, whether float `k` is on the right.
    pub fn float_is_right(&self, k: usize) -> bool {
        assert!(self.aside && k + 1 < self.count);
        self.float_right[k]
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
        aside: false,
        float_right: [false; MAX_FLOATS],
        dividers: [0.; MAX_COLUMNS - 1],
        divider_count: count - 1,
        divider_right: [false; 2],
        total,
    };
    let mut x = 0.;
    for (k, &share) in percent.iter().enumerate() {
        let width = usable * f32::from(share) / 100.;
        if width < MIN_COLUMN {
            return None;
        }
        (out.x[k], out.width[k]) = (x, width);
        if k > 0 {
            out.dividers[k - 1] = x - GAP / 2.;
        }
        x += width + GAP;
    }
    Some(out)
}

/// The widest share asked on each side, `(left, right)`: floats stacked down a
/// side are as wide as the widest of them.
fn side_shares(floats: &[FloatSpec]) -> (Option<u8>, Option<u8>) {
    let widest = |right: bool| {
        floats
            .iter()
            .filter(|f| f.right == right)
            .map(|f| f.share)
            .max()
    };
    (widest(false), widest(true))
}

/// The side widths `(left, right)` in pixels and the width the shares divide,
/// before any minimum is applied.
fn side_widths(total: f32, floats: &[FloatSpec]) -> (f32, f32, f32) {
    let (left, right) = side_shares(floats);
    let sides = usize::from(left.is_some()) + usize::from(right.is_some());
    let usable = total - GAP * sides as f32;
    let width = |share: Option<u8>| share.map_or(0., |s| usable * f32::from(s) / 100.);
    (width(left), width(right), usable)
}

/// The places of an aside's floats and of the text channel between them across
/// a line `total` pixels wide, or `None` when a side is too narrow or the
/// channel too: the region then stacks. Floats on one side are as wide as the
/// widest of them. The text keeps [`MIN_CHANNEL`]: floats asking for more than
/// that leaves are made narrower in proportion.
pub fn aside_split(total: f32, floats: &[FloatSpec]) -> Option<Split> {
    assert!((1..=MAX_FLOATS).contains(&floats.len()));
    if !total.is_finite() {
        return None;
    }
    let (mut left, mut right, usable) = side_widths(total, floats);
    let room = usable - MIN_CHANNEL;
    if left + right > room {
        if room <= 0. {
            return None;
        }
        let scale = room / (left + right);
        left *= scale;
        right *= scale;
    }
    let (has_left, has_right) = (left > 0., right > 0.);
    if (has_left && left < MIN_FLOAT) || (has_right && right < MIN_FLOAT) {
        return None;
    }
    let channel =
        total - GAP * (usize::from(has_left) + usize::from(has_right)) as f32 - left - right;
    if channel < MIN_COLUMN {
        return None;
    }
    let count = floats.len() + 1;
    let mut out = Split {
        count,
        x: [0.; MAX_COLUMNS],
        width: [0.; MAX_COLUMNS],
        aside: true,
        float_right: [false; MAX_FLOATS],
        dividers: [0.; MAX_COLUMNS - 1],
        divider_count: usize::from(has_left) + usize::from(has_right),
        divider_right: [false; 2],
        total,
    };
    for (k, f) in floats.iter().enumerate() {
        out.float_right[k] = f.right;
        (out.x[k], out.width[k]) = if f.right {
            (total - right, right)
        } else {
            (0., left)
        };
    }
    let channel_x = if has_left { left + GAP } else { 0. };
    (out.x[count - 1], out.width[count - 1]) = (channel_x, channel);
    let mut next = 0;
    if has_left {
        (out.dividers[next], out.divider_right[next]) = (left + GAP / 2., false);
        next += 1;
    }
    if has_right {
        (out.dividers[next], out.divider_right[next]) = (total - right - GAP / 2., true);
    }
    Some(out)
}

/// Where divider `k` of `region` would be for the shares `percent`, ignoring
/// every minimum width: a divider dragged so far that the region stacks still
/// has a place to be drawn and to be dragged back from.
pub fn divider_x_unchecked(total: f32, region: &Columns, percent: &[u8], k: usize) -> f32 {
    match region.kind {
        Kind::Aside => {
            let floats: Vec<FloatSpec> = region
                .floats()
                .iter()
                .zip(percent)
                .map(|(f, &share)| FloatSpec { share, ..*f })
                .collect();
            let (left, right, _) = side_widths(total, &floats);
            let (has_left, _) = side_shares(&floats);
            if has_left.is_some() && k == 0 {
                left + GAP / 2.
            } else {
                total - right - GAP / 2.
            }
        }
        Kind::Columns => {
            let usable = total - GAP * (percent.len() - 1) as f32;
            let before: f32 = percent[..=k]
                .iter()
                .map(|&p| usable * f32::from(p) / 100.)
                .sum();
            before + GAP * k as f32 + GAP / 2.
        }
    }
}

/// The left edge of column `k` and the width the shares divide, for shares
/// `percent` of columns, ignoring every minimum width (see
/// [`divider_x_unchecked`]).
pub fn column_left_unchecked(total: f32, percent: &[u8], k: usize) -> (f32, f32) {
    let usable = total - GAP * (percent.len() - 1) as f32;
    let before: f32 = percent[..k]
        .iter()
        .map(|&p| usable * f32::from(p) / 100.)
        .sum();
    (before + GAP * k as f32, usable)
}

/// The shares that put a stacked `region` back to columns: equal ones (every
/// float's default share for an aside), when they fit `total` pixels; `None`
/// when they would still stack, as in a window too narrow for the columns.
pub fn restored(region: &Columns, total: f32) -> Option<Vec<u8>> {
    match region.kind {
        Kind::Aside => {
            let floats: Vec<FloatSpec> = region
                .floats()
                .iter()
                .map(|f| FloatSpec {
                    share: ASIDE_DEFAULT,
                    ..*f
                })
                .collect();
            aside_split(total, &floats)?;
            let mut shares = vec![ASIDE_DEFAULT; floats.len()];
            shares.push(0);
            Some(shares)
        }
        Kind::Columns => {
            let equal = equal_shares(region.count)[..region.count].to_vec();
            split(total, &equal).map(|_| equal)
        }
    }
}

/// A float narrower than this is not worth wrapping text around.
pub const MIN_FLOAT: f32 = 120.;
/// The text beside floats keeps at least this width (about 55 characters):
/// narrower, greedy wrapping leaves rows 70-90% full and justifying them opens
/// wide gaps.
pub const MIN_CHANNEL: f32 = 560.;

//! The display model of one line: segments, styles, widgets, indent, and the
//! raw/display offset mapping.

use super::*;

/// How a visible [`DisplaySegment`] should be painted. Plain data —
/// `element.rs` maps each variant to an actual `Font`/color, keeping this
/// module GPUI-free. Meaningless (never read) on a hidden segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineStyle {
    Plain,
    Heading(u8),
    Bold,
    Italic,
    Code,
    Strikethrough,
    WikiLink,
    /// An inline link's visible text (`[text](url)`), or an autolink's
    /// visible URL/email (`<https://x>`, bare `https://x`) — the `(url)`/
    /// `<>` part is always hidden, never styled itself.
    Link,
    /// Content after a hidden blockquote `>` prefix; carries nesting depth
    /// (1 for `>`, 2 for `>>` or `> >`, ...).
    Blockquote(u8),
    /// The bullet/number glyph standing in for a hidden list marker, or a
    /// task-list checkbox glyph standing in for `[ ]`/`[x]` right after one
    /// — see [`DisplaySegment::is_replaced`]. Not applied to a list item's
    /// actual text content, only to these replacement glyphs.
    ListMarker,
    /// A callout header's title — either a custom title's own text (still
    /// inline-scanned, so it can itself contain `**bold**`/links/etc.) or
    /// the capitalized type name standing in for one, via
    /// [`build_callout_header_line`]. Carries no color/weight of its own
    /// here — `element.rs` resolves both from the line's actual
    /// [`callout::Callout`](super::callout::Callout), keeping this module
    /// GPUI-free.
    CalloutTitle,
    /// A control drawn as a 2D element over blank columns, never as a glyph
    /// from a font: see [`Widget`].
    Widget(Widget),
    /// A `$…$` formula `element.rs` draws from the bundled math font over the
    /// blank columns the segment reserves (see [`math::rendered_span`]); the
    /// segment's raw range is the span with its delimiters.
    Math,
}

/// A control `element.rs` paints with vector shapes or an icon over the
/// [`WIDGET_COLUMNS`] blank display columns its segment reserves, so wrapping,
/// hit-testing and the caret treat it like any replaced text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    /// An unordered list marker.
    Bullet,
    /// A task-list checkbox standing in for `[ ]`/`[x]`.
    Checkbox { checked: bool },
    /// A callout header's icon.
    CalloutIcon(CalloutType),
}

/// The blank text a widget's segment displays.
pub(super) const WIDGET_BLANK: &str = "  ";

/// One contiguous run of a line: the byte range it occupies in the *raw*
/// line, the byte range it occupies in [`DisplayLine::display_text`], and
/// the style to paint that display range in.
///
/// Two kinds contribute no *meaningful* raw↔display correspondence byte for
/// byte, and are treated alike by [`DisplayLine`]'s offset mapping (any raw
/// offset inside either one collapses to the segment's single display
/// point, since there's nothing finer to map to):
/// - **Hidden** markup (the `**`/`` ` ``/`[[`/... delimiters themselves, or
///   a list marker before replacement) — empty `display` range, paints
///   nothing. [`is_hidden`](Self::is_hidden).
/// - **Replaced** markup — non-empty `display` range, but one holding a
///   *different* string than the `raw` bytes it stands in for (a bullet/
///   number glyph, not the raw `-`/`1.` text) — so, unlike an ordinary
///   visible run, `display.len()` has no reason to equal `raw.len()` and a
///   byte offset inside it can't be interpolated. [`is_replaced`](Self::is_replaced).
///
/// Every other visible segment is shown verbatim (`display.len() ==
/// raw.len()`, the text itself unchanged from `raw`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySegment {
    pub raw: Range<usize>,
    pub display: Range<usize>,
    pub style: InlineStyle,
    pub(super) replaced: bool,
}

impl DisplaySegment {
    /// Markup collapsed to zero display width — paints nothing at all.
    pub fn is_hidden(&self) -> bool {
        self.display.is_empty()
    }

    /// A visible glyph standing in for different raw markup (see this
    /// struct's doc comment) — paints something, but not a verbatim slice
    /// of `raw`.
    pub fn is_replaced(&self) -> bool {
        self.replaced
    }
}

/// A line's structural indent — hanging indent for a list item, or nesting
/// depth for a blockquote — independent of whether its markup is currently
/// hidden. Unlike every `InlineStyle`, this applies even to the *active*
/// line: hiding `>`/`-` while editing is a Live Preview choice, but a quoted
/// or list line is still structurally a quote/list line while you're typing
/// it, so it should still indent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineIndent {
    None,
    /// Nesting depth (1 for `>`, 2 for `>>`/`> >`, ...) — same depth
    /// [`InlineStyle::Blockquote`] carries.
    Blockquote(u8),
    /// A list item at the given nesting depth (1 = top-level). A single
    /// line considered in isolation (see [`build_display_line`]) can't tell
    /// its own depth is anything but `1` — a *true* nested depth comes from
    /// [`classify_block_context`]'s whole-document pass, via
    /// [`build_display_line_in_context`].
    List(u8),
}

impl LineIndent {
    /// How many indent levels to shift the line by — `0` for `None`, the
    /// nesting depth otherwise (for both `Blockquote` and `List`).
    pub fn level(self) -> u8 {
        match self {
            LineIndent::None => 0,
            LineIndent::Blockquote(depth) | LineIndent::List(depth) => depth,
        }
    }
}

/// A line's raw Markdown text alongside what should actually be painted for
/// it: `display_text` (the hidden-markup-free, or unchanged-if-active,
/// string to shape), `segments` (the raw↔display mapping and per-run style
/// covering it), and `indent` (how far to shift the line right when
/// painting it — see [`LineIndent`]). `segments` are in raw order,
/// contiguous, and cover the whole line (`0..raw_line.len()`) with no gaps
/// or overlaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    pub display_text: String,
    pub segments: Vec<DisplaySegment>,
    pub indent: LineIndent,
}

impl DisplayLine {
    /// The heading level this line is shown as (a `#` heading, also inside a
    /// quote or list item), if it is one.
    pub fn heading_level(&self) -> Option<u8> {
        self.segments.iter().find_map(|s| match s.style {
            InlineStyle::Heading(level) => Some(level),
            _ => None,
        })
    }

    /// The factor the line's font size is scaled by: the `heading.scale` of its
    /// level, 1 for any other line.
    pub fn font_scale(&self) -> f32 {
        self.heading_level()
            .map_or(1.0, crate::settings::heading_scale)
    }

    /// How many body rows one row of this line takes (1 unless a heading).
    pub fn row_units(&self) -> usize {
        self.heading_level().map_or(1, |l| {
            crate::settings::heading_rows(crate::settings::heading_scale(l))
        })
    }

    /// Cuts the line at display byte `at` into the part before and the part
    /// from there on, and the raw byte where the second begins. Each part's
    /// raw and display ranges start at 0, so each reads as a line of its own.
    /// A replaced segment (a bullet glyph) cannot be cut, so the cut moves back
    /// to its start; a hidden one goes with the part after it. `at` is on a char
    /// boundary of `display_text`.
    pub fn split_at(&self, at: usize) -> (DisplayLine, DisplayLine, usize) {
        assert!(at <= self.display_text.len());
        let cut = self
            .segments
            .iter()
            .find(|s| s.is_replaced() && s.display.start < at && at < s.display.end)
            .map_or(at, |s| s.display.start);
        let mut first = Vec::new();
        let mut second = Vec::new();
        let mut raw_cut = None;
        for seg in &self.segments {
            if seg.display.end <= cut && !(seg.is_hidden() && seg.display.start == cut) {
                first.push(seg.clone());
            } else if seg.display.start >= cut {
                raw_cut.get_or_insert(seg.raw.start);
                second.push(seg.clone());
            } else {
                // A verbatim run the cut passes through: split both ranges.
                let into = cut - seg.display.start;
                let mut head = seg.clone();
                head.raw.end = seg.raw.start + into;
                head.display.end = cut;
                let mut tail = seg.clone();
                tail.raw.start = seg.raw.start + into;
                tail.display.start = cut;
                raw_cut = Some(tail.raw.start);
                first.push(head);
                second.push(tail);
            }
        }
        let raw_cut = raw_cut.unwrap_or_else(|| first.last().map_or(0, |s| s.raw.end));
        let rebase = |seg: &mut DisplaySegment| {
            seg.raw = seg.raw.start - raw_cut..seg.raw.end - raw_cut;
            seg.display = seg.display.start - cut..seg.display.end - cut;
        };
        second.iter_mut().for_each(rebase);
        let tail_text = self.display_text[cut..].to_owned();
        let head_text = self.display_text[..cut].to_owned();
        (
            DisplayLine {
                display_text: head_text,
                segments: first,
                indent: self.indent,
            },
            DisplayLine {
                display_text: tail_text,
                segments: second,
                indent: LineIndent::None,
            },
            raw_cut,
        )
    }

    /// The raw range of the task checkbox (`[ ]`/`[x]` and its space) whose
    /// glyph covers display offset `display_offset`.
    pub fn checkbox_at(&self, display_offset: usize) -> Option<Range<usize>> {
        let seg = self.segments.iter().find(|s| {
            s.display.contains(&display_offset)
                && matches!(s.style, InlineStyle::Widget(Widget::Checkbox { .. }))
        })?;
        Some(seg.raw.end - 4..seg.raw.end)
    }

    /// The display byte offset corresponding to raw byte offset `raw_offset`
    /// (which must be within this line's raw range). A `raw_offset` strictly
    /// inside a hidden *or* replaced segment (see [`DisplaySegment`]'s doc
    /// comment) collapses to that segment's display *start* — there's
    /// nothing finer to map it to, whether that's because the segment
    /// paints nothing (hidden) or because its text isn't a verbatim slice of
    /// `raw` to interpolate into (replaced). A `raw_offset` sitting exactly
    /// at such a segment's end resolves to its display *end* instead — for
    /// a zero-width hidden segment that's the same point as its start, but
    /// for a replaced segment (non-empty display range, e.g. a 4-byte "  "
    /// standing in for a 2-byte "- ") it's not: using `display.start` there
    /// would collapse back into the glyph instead of landing right after
    /// it, at the start of whatever visible text follows.
    pub fn display_offset_for_raw(&self, raw_offset: usize) -> usize {
        for seg in &self.segments {
            if raw_offset < seg.raw.end {
                return if seg.is_hidden() || seg.is_replaced() {
                    seg.display.start
                } else {
                    seg.display.start + (raw_offset - seg.raw.start)
                };
            }
            if raw_offset == seg.raw.end {
                return seg.display.end;
            }
        }
        self.display_text.len()
    }

    /// The raw byte offset corresponding to display byte offset
    /// `display_offset` — the inverse of [`display_offset_for_raw`]. Hidden
    /// and replaced segments are both skipped: a real `display_offset` (e.g.
    /// from a mouse click against shaped `display_text`) landing anywhere on
    /// either one — on the bullet glyph standing in for a list marker, say —
    /// falls through to the next segment via the same strict-`<` tie-break
    /// described below, landing at the start of whatever visible text
    /// follows it.
    ///
    /// Uses a strict `<` against each visible segment's end, not `<=`: when
    /// `display_offset` sits exactly on the boundary between a visible run
    /// and a hidden one right after it, that display point is shared by
    /// *two* raw offsets (just before the hidden markup, and just after it)
    /// — strict `<` resolves the tie toward the later one, landing past the
    /// hidden markup and at the start of whatever visible text follows it,
    /// which is what a click right at that boundary should feel like.
    pub fn raw_offset_for_display(&self, display_offset: usize) -> usize {
        for seg in &self.segments {
            if seg.is_hidden() || seg.is_replaced() {
                continue;
            }
            if display_offset < seg.display.end {
                let delta = display_offset.saturating_sub(seg.display.start);
                return seg.raw.start + delta;
            }
        }
        self.segments.last().map_or(0, |s| s.raw.end)
    }

    /// Like [`raw_offset_for_display`](Self::raw_offset_for_display), but
    /// considers *every* segment, including a hidden or replaced one,
    /// rather than skipping straight past it to whatever comes next —
    /// needed for a table's wrapped sub-rows, where
    /// *every* segment (even a cell's own text) is `is_replaced()` once
    /// that cell's content no longer fits on one line, so
    /// `raw_offset_for_display`'s "skip every replaced segment" rule would
    /// otherwise always fall through to this line's very end, regardless
    /// of where it was actually clicked. Every other construct in this
    /// editor keeps at least one genuinely visible segment to land on, so
    /// it never needs this — it's specific to that one table case.
    pub fn nearest_segment_raw_start(&self, display_offset: usize) -> usize {
        for seg in &self.segments {
            if display_offset < seg.display.end {
                return seg.raw.start;
            }
        }
        self.segments.last().map_or(0, |s| s.raw.start)
    }
}

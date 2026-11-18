//! Per-line Live Preview rendering model: how a note's *raw* Markdown line
//! maps to what should actually be painted — hidden markup (heading `#`
//! prefixes, `**`/`*` emphasis delimiters, `` ` `` code fences, `[[`/`]]`/
//! `|target` wikilink syntax) collapsed to zero display width, the remaining
//! text tagged with the style it should be painted in.
//!
//! Deliberately GPUI-free (same pure-logic-first approach `buffer.rs` and
//! every other Live Preview module use) — `element.rs`/`input_handler.rs`
//! consume [`DisplaySegment::style`] to pick an actual `Font`/color, and its
//! raw↔display byte mapping ([`DisplayLine::display_offset_for_raw`]/
//! [`DisplayLine::raw_offset_for_display`]) to convert between a raw buffer
//! offset and an x position in the *display* text they actually shape and
//! paint — the two are no longer the same string once markup is hidden.
//!
//! Scope deliberately narrow for this first pass: headings, bold, italic,
//! inline code, strikethrough, wikilinks, blockquote `>` prefixes, list
//! markers, and tables (`table_preview.rs`'s detection, this module's
//! `table_box` rendering) — not the full
//! CommonMark/GFM grammar (no autolinks). Emphasis matching is also
//! simplified: `**x**`/`__x__` is bold, a lone `*x*`/`_x_` is italic,
//! without CommonMark's full left/right-flanking delimiter-run algorithm —
//! a deliberate v1 scope cut. Strikethrough (`~~x~~`) only recognizes the
//! doubled-tilde GFM form, mirroring Strong's `**`/`__` pairing — unlike GitHub's own
//! renderer, a lone `~x~` isn't treated as anything special and is left as
//! plain text, since GFM itself doesn't define single-tilde strikethrough
//! either (that's a cmark-gfm extension option GitHub doesn't enable).
//!
//! Most detection in this module (headings, blockquote `>` prefixes, list
//! markers, one level of compounding) is fully determined by a single
//! line's own content, needing no whole-document state — unlike fenced code
//! (`block_preview.rs`'s own concern; whether a line sits inside an open
//! fence depends on earlier lines). Two things *do* need cross-line state
//! though: an indented list marker's nesting *depth* (relative to
//! whichever ancestor items are still open), and whether a line with no
//! marker of its own still continues an open blockquote/list (CommonMark
//! "lazy continuation"). [`classify_block_context`] is this module's own
//! single sequential pass resolving both — the same kind of whole-document
//! state machine `block_preview::classify_lines` already uses for fences,
//! just tracking list-indentation depth and open-block state instead of
//! fence delimiters. `element.rs`'s `render_editor` computes it once per
//! paint (like `classify_lines`) and feeds each line's resulting
//! [`LineContext`] into [`build_display_line_in_context`] instead of
//! calling [`build_display_line`] directly — the latter, being pure
//! per-line, has no way to know a marker is nested or a line is a lazy
//! continuation.
//!
//! List markers are hidden like any other markup, but — unlike a `**`/`#`,
//! which simply vanishes — are *replaced* with a bullet (`•`) or a
//! canonical `N. `, not left blank: see [`DisplaySegment::is_replaced`].
//!
//! The line containing the cursor is never hidden — `build_display_line`'s
//! `active` flag turns rendering off entirely for it, showing the literal
//! raw text unstyled. (A "real" Live Preview editor keeps *formatting*
//! visible while editing and hides only the delimiter characters; doing that
//! too is a reasonable follow-up, not required for this first pass given the
//! narrow construct list above.)

use std::ops::Range;
use std::rc::Rc;

use super::callout::{self, callout_header_prefix};
use jade_core::link_scan::{
    find_closing, parse_autolink, parse_bare_url, parse_inline_link, parse_wikilink,
};

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
}

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
    replaced: bool,
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
    /// The display byte offset corresponding to raw byte offset `raw_offset`
    /// (which must be within this line's raw range). A `raw_offset` strictly
    /// inside a hidden *or* replaced segment (see [`DisplaySegment`]'s doc
    /// comment) collapses to that segment's display *start* — there's
    /// nothing finer to map it to, whether that's because the segment
    /// paints nothing (hidden) or because its text isn't a verbatim slice of
    /// `raw` to interpolate into (replaced). A `raw_offset` sitting exactly
    /// at such a segment's end resolves to its display *end* instead — for
    /// a zero-width hidden segment that's the same point as its start, but
    /// for a replaced segment (non-empty display range, e.g. a 4-byte "• "
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

/// Builds the Live Preview rendering of one `\n`-free `line`, considered in
/// isolation: unchanged if `active` (the line containing the cursor) or
/// empty, else with headings/bold/italic/inline-code/wikilink markup hidden
/// per this module's doc comment. `indent` is computed from `line`'s own
/// prefix regardless of `active` — see [`LineIndent`]'s doc comment for why
/// it applies even to the raw-rendered active line. A list marker here is
/// always resolved to depth `1` (an indented one still has its leading
/// whitespace and marker hidden, via [`indented_list_marker`], but this
/// function alone can't know its *true* nesting depth — that needs
/// surrounding lines, see [`build_display_line_in_context`]).
///
/// A blockquote or list item's own content gets exactly one further look
/// for a *compounding* construct — a heading, or (whichever of
/// blockquote/list the outer one wasn't) the other — via [`compound_inner`],
/// so e.g. `> # Heading` hides both the `> ` and the `# `, not just the
/// former. See [`compound_inner`]'s own doc comment for what's still out of
/// scope (list-in-list compounding on one line, e.g. unindented `- - x`;
/// and compounding more than one level deep).
pub fn build_display_line(line: &str, active: bool) -> DisplayLine {
    let indent = line_indent(line);

    if active || line.is_empty() {
        return DisplayLine {
            display_text: line.to_string(),
            segments: vec![DisplaySegment {
                raw: 0..line.len(),
                display: 0..line.len(),
                style: InlineStyle::Plain,
                replaced: false,
            }],
            indent,
        };
    }

    let mut segments = Vec::new();
    let mut display_text = String::new();

    if let Some((level, prefix_len)) = heading_prefix(line) {
        push_hidden(&mut segments, &display_text, 0..prefix_len);
        scan_inline(
            line,
            prefix_len,
            line.len(),
            InlineStyle::Heading(level),
            &mut segments,
            &mut display_text,
        );
        return DisplayLine {
            display_text,
            segments,
            indent,
        };
    }

    if let Some((depth, prefix_len)) = blockquote_prefix(line) {
        push_hidden(&mut segments, &display_text, 0..prefix_len);
        let (content_start, style) = compound_inner(
            line,
            prefix_len,
            true,
            false,
            InlineStyle::Blockquote(depth),
            &mut segments,
            &mut display_text,
        );
        scan_inline(
            line,
            content_start,
            line.len(),
            style,
            &mut segments,
            &mut display_text,
        );
        return DisplayLine {
            display_text,
            segments,
            indent,
        };
    }

    if let Some((marker, leading_ws, marker_len)) = indented_list_marker(line) {
        push_hidden(&mut segments, &display_text, 0..leading_ws);
        let marker_end = push_list_marker(
            line,
            leading_ws,
            &marker,
            marker_len,
            &mut segments,
            &mut display_text,
        );
        let (content_start, style) = compound_inner(
            line,
            marker_end,
            false,
            true,
            InlineStyle::Plain,
            &mut segments,
            &mut display_text,
        );
        scan_inline(
            line,
            content_start,
            line.len(),
            style,
            &mut segments,
            &mut display_text,
        );
        return DisplayLine {
            display_text,
            segments,
            indent,
        };
    }

    scan_inline(
        line,
        0,
        line.len(),
        InlineStyle::Plain,
        &mut segments,
        &mut display_text,
    );
    DisplayLine {
        display_text,
        segments,
        indent,
    }
}

/// A table cell's text with its inline markup applied (bold, code, links), as
/// one line. Unlike [`build_display_line`] it never reads a leading `>`, `-` or
/// `#` as a block: inside a cell they are just characters.
pub fn build_cell_line(text: &str) -> DisplayLine {
    let mut segments = Vec::new();
    let mut display_text = String::new();
    scan_inline(
        text,
        0,
        text.len(),
        InlineStyle::Plain,
        &mut segments,
        &mut display_text,
    );
    DisplayLine {
        display_text,
        segments,
        indent: LineIndent::None,
    }
}

/// Checks `line[offset..]` (the content right after an outer blockquote `>`
/// or list marker the caller already hid/replaced) for one further level of
/// compounding: a heading (always checked — a heading never itself compounds
/// further, see the module doc), then, if `check_list`/`check_blockquote`,
/// a list marker or blockquote prefix. Pushes whatever hidden/replaced
/// segment the match needs and returns the byte offset inline scanning
/// should resume from, plus the `InlineStyle` to scan with. Falls back to
/// `(offset, fallback)` — the outer construct's own style — if nothing
/// compounds.
///
/// Only one of `check_list`/`check_blockquote` is ever `true` for a given
/// call: a blockquote's content may compound with a list (not another
/// blockquote — multiple `>`s are already one `blockquote_prefix` match,
/// see [`line_indent`]), and a list item's content may compound with a
/// blockquote (not another list marker — an *indented* nested list is
/// recognized, but only across lines via [`classify_block_context`]'s
/// whole-document pass, not within one line's own compounding check; an
/// unindented second marker right after the first, e.g. `- - x`, is just
/// plain text starting with `-`, per CommonMark itself — list nesting is
/// signaled by indentation, not by repeating the marker).
fn compound_inner(
    line: &str,
    offset: usize,
    check_list: bool,
    check_blockquote: bool,
    fallback: InlineStyle,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
) -> (usize, InlineStyle) {
    let rest = &line[offset..];

    if let Some((level, inner_len)) = heading_prefix(rest) {
        push_hidden(segments, display_text, offset..offset + inner_len);
        return (offset + inner_len, InlineStyle::Heading(level));
    }

    if check_list && let Some((marker, marker_len)) = list_marker(rest) {
        let content_start =
            push_list_marker(line, offset, &marker, marker_len, segments, display_text);
        return (content_start, InlineStyle::Plain);
    }

    if check_blockquote && let Some((depth, inner_len)) = blockquote_prefix(rest) {
        push_hidden(segments, display_text, offset..offset + inner_len);
        return (offset + inner_len, InlineStyle::Blockquote(depth));
    }

    (offset, fallback)
}

/// `line`'s structural indent (see [`LineIndent`]), from the same
/// `blockquote_prefix`/`list_marker` detection `build_display_line` uses for
/// hiding/replacing markup — computed separately (rather than threaded out
/// of those branches) so it's available even on the active line, which skips
/// them entirely.
pub(super) fn line_indent(line: &str) -> LineIndent {
    if let Some((depth, _)) = blockquote_prefix(line) {
        return LineIndent::Blockquote(depth);
    }
    if indented_list_marker(line).is_some() {
        return LineIndent::List(1);
    }
    LineIndent::None
}

/// What [`classify_block_context`] resolved a line to need, beyond what
/// [`build_display_line`] can tell from that line alone — consumed by
/// [`build_display_line_in_context`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineContext {
    /// Nothing from whole-document context is needed — `build_display_line`
    /// already resolves this line correctly by itself (not part of any
    /// list/blockquote, or it has its own unambiguous `>`/marker prefix).
    Plain,
    /// An indented list marker line, at `depth` (1 = top-level) resolved
    /// from its indentation relative to whichever ancestor items were still
    /// open — `build_display_line` alone would assume depth `1` regardless
    /// of indentation (see its own doc comment).
    ListMarkerLine(u8),
    /// A line with no blockquote/list prefix of its own, still treated as
    /// continuing the enclosing block's content (CommonMark "lazy
    /// continuation") at this [`LineIndent`].
    LazyContinuation(LineIndent),
}

/// Resolves, for every line of the whole `source` document, the two things
/// [`build_display_line`] can't tell from a single line in isolation:
/// nested-list depth and lazy continuation — see this module's own doc
/// comment for why both need cross-line state, unlike everything else this
/// module detects. A single sequential pass (mirroring
/// `block_preview::classify_lines`'s fence-tracking state machine), tracking
/// whichever block is currently "open":
///
/// - A blockquote `>` line opens [`LineIndent::Blockquote`] at its depth.
/// - An (optionally indented) list marker line opens or continues a stack of
///   nesting levels, keyed by each level's own indentation column: deeper
///   levels (indented further than the current marker) are popped, a level
///   at the *same* column is reused (a sibling item, not a new level), and
///   anything strictly shallower stays open as this marker's ancestor.
/// - A blank line or a heading (CommonMark: both always interrupt an open
///   paragraph) closes whatever was open.
/// - Any other non-blank line with no marker of its own is a lazy
///   continuation of whatever's still open, *regardless of its own
///   indentation* — deliberately as lenient as a blockquote's lazy
///   continuation already is, not CommonMark's stricter per-list-item
///   content-column alignment rule, since this module doesn't track each
///   level's content-start column, only where its marker began.
///
/// `document_cache.rs`'s production path calls `classify_block_context_from`
/// directly (it needs the per-line states too, to seed its own cache) — this
/// whole-document entry point is only still reachable from tests now, kept
/// as the oracle every classify_block_context_from-based test (including
/// the incremental path's own) checks itself against, same role
/// `block_preview::classify_lines`/`callout::classify_callouts` now play
/// there.
#[cfg(test)]
pub(super) fn classify_block_context(source: &str) -> Vec<LineContext> {
    classify_block_context_from(source, OpenBlock::None)
        .into_iter()
        .map(|(ctx, _)| ctx)
        .collect()
}

/// The cross-line state [`block_context_step`] carries — see
/// [`classify_block_context`]'s own doc comment for what each variant
/// means. Pulled out to module scope (rather than kept as a function-local
/// `enum`, which is what it originally was) so an incremental caller
/// (`document_cache.rs`) can store and compare it across paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OpenBlock {
    None,
    Blockquote(u8),
    /// The currently-open list's nesting stack, top = innermost level —
    /// see [`ListNode`]'s own doc comment for why this is a persistent
    /// `Rc`-linked chain rather than a `Vec<usize>`. Never `None` while
    /// this variant is current — see the marker-line branch below, which
    /// always leaves at least one level.
    List(Rc<ListNode>),
}

/// One level of [`OpenBlock::List`]'s nesting stack, linked to its parent
/// (the next level out) via `Rc` rather than stored inline in a `Vec` —
/// replaces what used to be a plain `Vec<usize>` that `classify_block_
/// context_from`'s loop deep-cloned (`open = next.clone()`) on *every*
/// line inside an open list, not just where nesting depth actually
/// changed (confirmed by direct measurement: ~18MB / 98,642 separate
/// heap allocations on the 320K-line stress fixture, where 21.9% of all
/// lines are inside some open list — see HISTORY.md).
///
/// The `Rc`-chain representation makes the *common* case genuinely free:
/// a sibling item at the same depth, or any plain non-marker line,
/// reuses the existing top node via a cheap `Rc::clone()` (no allocation
/// at all), and even a real depth *increase* only ever allocates one
/// small node (not a whole stack) — see [`ListNode::resume_or_push`].
/// `depth` is cached here (rather than derived by walking `parent`) so
/// reading it stays O(1), matching what `Vec::len()` already was for the
/// structure this replaced.
///
/// `PartialEq`/`Eq` are derived, not hand-written, and still do the right
/// thing for `document_cache.rs`'s reconvergence check (which needs two
/// *separately allocated* chains representing the same sequence of
/// indents to compare equal, the same as two `Vec<usize>`s with identical
/// contents always did): `Rc<T>`'s own `PartialEq` impl already compares
/// *values* (`**self == **other`), falling back to that whenever the
/// pointers themselves differ, rather than only ever comparing pointer
/// identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ListNode {
    indent: usize,
    depth: u8,
    parent: Option<Rc<ListNode>>,
}

impl ListNode {
    /// Resolves the list-nesting stack's new top node for a marker line at
    /// `indent`, given `current` (the stack's existing top, or `None` if
    /// no list was open) — the structural-sharing replacement for the
    /// original `Vec<usize>`'s "pop while deeper, then push if not already
    /// at this depth" dance. Walks up `parent` links (no allocation) while
    /// the current top is deeper than `indent`; if it lands exactly on
    /// `indent` (a sibling item, or dedenting back to an already-open
    /// ancestor level), that existing node is reused outright — only an
    /// actual depth *increase* allocates, and only one small node.
    fn resume_or_push(current: Option<Rc<ListNode>>, indent: usize) -> Rc<ListNode> {
        let mut node = current;
        while let Some(n) = &node {
            if n.indent > indent {
                node = n.parent.clone();
            } else {
                break;
            }
        }
        match node {
            Some(n) if n.indent == indent => n,
            other => {
                let depth = other.as_ref().map_or(1, |n| n.depth + 1);
                Rc::new(ListNode {
                    indent,
                    depth,
                    parent: other,
                })
            }
        }
    }
}

/// `line.trim().is_empty()`, deciding from the first byte when it is a
/// printable ASCII character (nearly every line) before the Unicode-aware
/// trim.
pub(super) fn is_blank(line: &str) -> bool {
    match line.as_bytes().first() {
        None => true,
        Some(&b) if b > b' ' && b < 0x7f => false,
        Some(_) => line.trim().is_empty(),
    }
}

/// The one per-line transition [`classify_block_context`]/
/// [`classify_block_context_from`] both fold over — pulled out of the
/// original single sequential loop the same way `block_preview::step`/
/// `callout::step` were, so an incremental caller can resume from an
/// arbitrary line's cached entering state instead of only ever starting
/// fresh from line 0.
pub(super) fn block_context_step(open: OpenBlock, line: &str) -> (LineContext, OpenBlock) {
    if is_blank(line) {
        return (LineContext::Plain, OpenBlock::None);
    }

    if heading_prefix(line).is_some() {
        return (LineContext::Plain, OpenBlock::None);
    }

    if let Some((depth, _)) = blockquote_prefix(line) {
        return (LineContext::Plain, OpenBlock::Blockquote(depth));
    }

    if let Some((_, indent, _)) = indented_list_marker(line) {
        let current = match open {
            OpenBlock::List(top) => Some(top),
            _ => None,
        };
        let top = ListNode::resume_or_push(current, indent);
        let depth = top.depth;
        return (LineContext::ListMarkerLine(depth), OpenBlock::List(top));
    }

    let ctx = match &open {
        OpenBlock::Blockquote(depth) => {
            LineContext::LazyContinuation(LineIndent::Blockquote(*depth))
        }
        OpenBlock::List(top) => LineContext::LazyContinuation(LineIndent::List(top.depth)),
        OpenBlock::None => LineContext::Plain,
    };
    (ctx, open)
}

#[cfg(test)]
/// Resumes classification from `open` (the state *entering* the first line
/// of `source`), which is normally a suffix of the whole document starting
/// at some line, not necessarily the document's actual first line. Returns
/// each line's [`LineContext`] paired with the state *after* it, so an
/// incremental caller can compare that against a previously cached
/// entering state for the corresponding later line to know the moment it's
/// safe to stop and reuse the old cached suffix unchanged — see
/// `document_cache.rs`.
pub(super) fn classify_block_context_from(
    source: &str,
    open: OpenBlock,
) -> Vec<(LineContext, OpenBlock)> {
    let mut open = open;
    source
        .split('\n')
        .map(|line| {
            let current = std::mem::replace(&mut open, OpenBlock::None);
            let (ctx, next) = block_context_step(current, line);
            open = next.clone();
            (ctx, next)
        })
        .collect()
}

/// [`build_display_line`], but overridden by whole-document context
/// ([`LineContext`], from [`classify_block_context`]) for the two things
/// that function can't resolve from `line` alone:
/// - [`LineContext::ListMarkerLine`]: same as `build_display_line`, except
///   the resulting [`LineIndent::List`] carries the real nested depth
///   instead of always `1`.
/// - [`LineContext::LazyContinuation`]: `line` has no blockquote/list prefix
///   of its own (so `build_display_line` would treat it as an ordinary
///   paragraph), but still gets `indent`'s shift and — unless `active` —
///   its *entire* text styled as that block's content (`Blockquote(depth)`,
///   or `Plain` for a list continuation, matching what a real prefix line's
///   own content would be styled as).
pub fn build_display_line_in_context(line: &str, active: bool, ctx: LineContext) -> DisplayLine {
    match ctx {
        LineContext::Plain => build_display_line(line, active),
        LineContext::ListMarkerLine(depth) => {
            let mut display_line = build_display_line(line, active);
            display_line.indent = LineIndent::List(depth);
            display_line
        }
        LineContext::LazyContinuation(indent) => {
            if active || line.is_empty() {
                return DisplayLine {
                    display_text: line.to_string(),
                    segments: vec![DisplaySegment {
                        raw: 0..line.len(),
                        display: 0..line.len(),
                        style: InlineStyle::Plain,
                        replaced: false,
                    }],
                    indent,
                };
            }
            let style = match indent {
                LineIndent::Blockquote(depth) => InlineStyle::Blockquote(depth),
                LineIndent::List(_) | LineIndent::None => InlineStyle::Plain,
            };
            let mut segments = Vec::new();
            let mut display_text = String::new();
            scan_inline(line, 0, line.len(), style, &mut segments, &mut display_text);
            DisplayLine {
                display_text,
                segments,
                indent,
            }
        }
    }
}

/// An ATX heading's level and the byte length of its hidden prefix (the
/// `#`×level marker plus the whitespace run right after it), if `line`
/// starts with one. CommonMark's allowance for up to 3 leading spaces before
/// the `#`s isn't handled — a line's own leading whitespace isn't something
/// this editor strips elsewhere either.
fn heading_prefix(line: &str) -> Option<(u8, usize)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let bytes = line.as_bytes();
    if bytes.get(hashes) != Some(&b' ') {
        return None;
    }
    let mut end = hashes;
    while bytes.get(end) == Some(&b' ') {
        end += 1;
    }
    Some((hashes as u8, end))
}

/// Whether `line` is a thematic break (`---`, `***`, `___`, optionally with
/// spaces between the repeated characters, e.g. `- - -`): every non-space
/// byte the same one of `-`/`*`/`_`, with at least 3 of them. Block-level
/// and resolved from a single line alone, unlike fenced code/tables — see
/// this module's doc comment. Checked by `element.rs` (`shape_display_line`/
/// `paint_lines`) only for a `LineRole::Plain` line outside any table, so a
/// `---` inside a fenced code block or a table's own delimiter row (which
/// has `|` characters mixed in, failing the "all one char" check here
/// anyway) is never mistaken for one.
pub fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    let mut marker: Option<u8> = None;
    let mut count = 0;
    for b in trimmed.bytes() {
        if b == b' ' {
            continue;
        }
        if !matches!(b, b'-' | b'*' | b'_') {
            return false;
        }
        match marker {
            Some(m) if m != b => return false,
            Some(_) => {}
            None => marker = Some(b),
        }
        count += 1;
    }
    count >= 3
}

enum ConstructKind {
    Code,
    Strong,
    Emphasis,
    Strikethrough,
    WikiLink,
    /// `\` followed by an ASCII punctuation character — a one-shot match
    /// (no closer to search for), handled directly in `scan_inline`.
    Escape,
    /// `&name;` / `&#NNN;` / `&#xHHH;` — a one-shot match like `Escape`,
    /// resolved by [`parse_entity`].
    Entity,
    /// `[text](url)` — resolved by [`parse_inline_link`]. Not matched at all when
    /// immediately preceded by `!` (an image, deliberately out of scope —
    /// see `scan_inline`'s own handling).
    Link,
    /// `<https://x>` / `<mailto:a@b.com>` / `<a@b.com>` — resolved by
    /// [`parse_autolink`].
    AutolinkBracket,
    /// A bare `http://`/`https://` URL with no surrounding delimiters (GFM
    /// extended autolinks) — a one-shot match, resolved directly in
    /// `scan_inline` since there's no closer to search for.
    BareUrl,
}

struct Construct {
    /// Byte offset of the opener, relative to the slice `find_construct` was
    /// called on (the caller adds its own base offset back in).
    offset: usize,
    opener_len: usize,
    kind: ConstructKind,
}

/// The next inline construct's opener in `rest`, if any — a backtick, a
/// doubled or single `*`/`_` run, a doubled `~` run, `[[`, a backslash escape,
/// an entity's `&`, a lone `[`, `<`, or a bare `http(s)://`. A lone `~` (not
/// doubled) isn't a construct at all — unlike `*`/`_`, which have both a
/// doubled (strong) and single (emphasis) form, strikethrough only
/// recognizes the doubled form, so a single `~` is simply skipped over, same
/// as any other plain character. Scans byte-by-byte; safe over multi-byte
/// UTF-8 since none of its continuation bytes can match any of these ASCII
/// patterns, so stepping through one at a time just finds nothing until the
/// next real ASCII delimiter.
fn find_construct(rest: &str) -> Option<Construct> {
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::Escape,
                });
            }
            b'`' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Code,
                });
            }
            b'*' | b'_' => {
                let c = bytes[i];
                return Some(if bytes.get(i + 1) == Some(&c) {
                    Construct {
                        offset: i,
                        opener_len: 2,
                        kind: ConstructKind::Strong,
                    }
                } else {
                    Construct {
                        offset: i,
                        opener_len: 1,
                        kind: ConstructKind::Emphasis,
                    }
                });
            }
            b'~' if bytes.get(i + 1) == Some(&b'~') => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::Strikethrough,
                });
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                return Some(Construct {
                    offset: i,
                    opener_len: 2,
                    kind: ConstructKind::WikiLink,
                });
            }
            b'[' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Link,
                });
            }
            b'<' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::AutolinkBracket,
                });
            }
            b'&' => {
                return Some(Construct {
                    offset: i,
                    opener_len: 1,
                    kind: ConstructKind::Entity,
                });
            }
            b'h' if rest[i..].starts_with("http://") || rest[i..].starts_with("https://") => {
                return Some(Construct {
                    offset: i,
                    opener_len: 0,
                    kind: ConstructKind::BareUrl,
                });
            }
            _ => i += 1,
        }
    }
    None
}

/// Common named HTML entities this module decodes — not the full ~2000-entry
/// HTML5 named character reference table (a deliberate v1 scope cut, same
/// category as strikethrough's GFM-only doubled-tilde): just a subset
/// plausible in a note's prose.
const NAMED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{00A0}'),
    ("mdash", '\u{2014}'),
    ("ndash", '\u{2013}'),
    ("hellip", '\u{2026}'),
    ("copy", '\u{00A9}'),
    ("reg", '\u{00AE}'),
    ("trade", '\u{2122}'),
    ("laquo", '\u{00AB}'),
    ("raquo", '\u{00BB}'),
    ("ldquo", '\u{201C}'),
    ("rdquo", '\u{201D}'),
    ("lsquo", '\u{2018}'),
    ("rsquo", '\u{2019}'),
    ("deg", '\u{00B0}'),
    ("plusmn", '\u{00B1}'),
    ("times", '\u{00D7}'),
    ("divide", '\u{00F7}'),
    ("sect", '\u{00A7}'),
    ("para", '\u{00B6}'),
    ("bull", '\u{2022}'),
    ("euro", '\u{20AC}'),
    ("pound", '\u{00A3}'),
    ("yen", '\u{00A5}'),
    ("cent", '\u{00A2}'),
];

/// Parses an HTML entity reference (`&amp;`, `&#39;`, `&#x27;`) starting at
/// `start` (pointing at `&`), if `line[start..end]` contains a recognized
/// one — returns its raw span (including `&`/`;`) and the decoded
/// character. Numeric references decode any valid Unicode scalar value;
/// named references are looked up in [`NAMED_ENTITIES`]. An unrecognized
/// name or invalid numeric value is `None` — the usual "leave it as plain
/// text" fallback every other unmatched construct in this module gets, so
/// e.g. a bare `&` in "Bob & Alice" is untouched.
fn parse_entity(line: &str, start: usize, end: usize) -> Option<(Range<usize>, char)> {
    let rest = &line[start..end];
    let semi_rel = rest.find(';')?;
    let body = &rest[1..semi_rel];
    let raw_end = start + semi_rel + 1;

    let ch = if let Some(hex) = body
        .strip_prefix('#')
        .and_then(|b| b.strip_prefix('x').or_else(|| b.strip_prefix('X')))
    {
        if hex.is_empty() || hex.len() > 6 {
            return None;
        }
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if let Some(dec) = body.strip_prefix('#') {
        if dec.is_empty() || dec.len() > 7 {
            return None;
        }
        char::from_u32(dec.parse().ok()?)?
    } else {
        NAMED_ENTITIES
            .iter()
            .find(|(name, _)| *name == body)
            .map(|(_, c)| *c)?
    };
    Some((start..raw_end, ch))
}

/// Scans `line[pos..end]` for inline constructs, appending hidden/visible
/// [`DisplaySegment`]s (and the visible text they show) in raw order. Plain
/// text outside any construct is tagged `default_style` — `Plain` normally,
/// or the enclosing heading's level so e.g. `# **bold** title` still renders
/// its non-bold text at heading size/color.
fn scan_inline(
    line: &str,
    mut pos: usize,
    end: usize,
    default_style: InlineStyle,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
) {
    let mut plain_start = pos;
    while pos < end {
        let Some(construct) = find_construct(&line[pos..end]) else {
            break;
        };
        let construct_start = pos + construct.offset;

        push_visible(
            line,
            segments,
            display_text,
            plain_start..construct_start,
            default_style,
        );
        plain_start = construct_start;

        let matched = match construct.kind {
            ConstructKind::Code => find_closing(line, construct_start + 1, end, "`").map(|close| {
                push_hidden(segments, display_text, construct_start..construct_start + 1);
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start + 1..close,
                    InlineStyle::Code,
                );
                push_hidden(segments, display_text, close..close + 1);
                close + 1
            }),
            ConstructKind::Strong | ConstructKind::Emphasis => {
                let delim_len = construct.opener_len;
                let delim = &line[construct_start..construct_start + delim_len];
                let style = if matches!(construct.kind, ConstructKind::Strong) {
                    InlineStyle::Bold
                } else {
                    InlineStyle::Italic
                };
                find_closing(line, construct_start + delim_len, end, delim).map(|close| {
                    push_hidden(
                        segments,
                        display_text,
                        construct_start..construct_start + delim_len,
                    );
                    push_visible(
                        line,
                        segments,
                        display_text,
                        construct_start + delim_len..close,
                        style,
                    );
                    push_hidden(segments, display_text, close..close + delim_len);
                    close + delim_len
                })
            }
            ConstructKind::Strikethrough => {
                find_closing(line, construct_start + 2, end, "~~").map(|close| {
                    push_hidden(segments, display_text, construct_start..construct_start + 2);
                    push_visible(
                        line,
                        segments,
                        display_text,
                        construct_start + 2..close,
                        InlineStyle::Strikethrough,
                    );
                    push_hidden(segments, display_text, close..close + 2);
                    close + 2
                })
            }
            ConstructKind::WikiLink => parse_wikilink(line, construct_start, end).map(|link| {
                push_hidden(segments, display_text, construct_start..link.text.start);
                push_visible(
                    line,
                    segments,
                    display_text,
                    link.text.clone(),
                    InlineStyle::WikiLink,
                );
                push_hidden(segments, display_text, link.text.end..link.span.end);
                link.span.end
            }),
            ConstructKind::Escape => {
                push_hidden(segments, display_text, construct_start..construct_start + 1);
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start + 1..construct_start + 2,
                    default_style,
                );
                Some(construct_start + 2)
            }
            ConstructKind::Entity => parse_entity(line, construct_start, end).map(|(raw, ch)| {
                let mut buf = [0u8; 4];
                let decoded = ch.encode_utf8(&mut buf);
                push_replaced(segments, display_text, raw.clone(), decoded, default_style);
                raw.end
            }),
            // `![...]` is an image, explicitly deferred (see this module's
            // doc comment) — left as plain text, same as any other
            // unmatched opener.
            ConstructKind::Link
                if construct_start > 0 && line.as_bytes()[construct_start - 1] == b'!' =>
            {
                None
            }
            ConstructKind::Link => parse_inline_link(line, construct_start, end).map(|link| {
                push_hidden(segments, display_text, construct_start..link.text.start);
                push_visible(
                    line,
                    segments,
                    display_text,
                    link.text.clone(),
                    InlineStyle::Link,
                );
                push_hidden(segments, display_text, link.text.end..link.span.end);
                link.span.end
            }),
            ConstructKind::AutolinkBracket => {
                parse_autolink(line, construct_start, end).map(|link| {
                    push_hidden(segments, display_text, construct_start..link.text.start);
                    push_visible(
                        line,
                        segments,
                        display_text,
                        link.text.clone(),
                        InlineStyle::Link,
                    );
                    push_hidden(segments, display_text, link.text.end..link.span.end);
                    link.span.end
                })
            }
            ConstructKind::BareUrl => {
                let url_end = parse_bare_url(line, construct_start, end).span.end;
                push_visible(
                    line,
                    segments,
                    display_text,
                    construct_start..url_end,
                    InlineStyle::Link,
                );
                Some(url_end)
            }
        };

        match matched {
            Some(new_pos) => {
                pos = new_pos;
                plain_start = pos;
            }
            // No closer on this line: leave the opener as plain text (still
            // unflushed, in `plain_start`) and keep scanning right after it.
            None => pos = construct_start + construct.opener_len,
        }
    }
    push_visible(
        line,
        segments,
        display_text,
        plain_start..end,
        default_style,
    );
}

fn push_hidden(segments: &mut Vec<DisplaySegment>, display_text: &str, raw: Range<usize>) {
    if raw.is_empty() {
        return;
    }
    let at = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: at..at,
        style: InlineStyle::Plain,
        replaced: false,
    });
}

fn push_visible(
    line: &str,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
    raw: Range<usize>,
    style: InlineStyle,
) {
    if raw.is_empty() {
        return;
    }
    let display_start = display_text.len();
    display_text.push_str(&line[raw.clone()]);
    let display_end = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: display_start..display_end,
        style,
        replaced: false,
    });
}

/// Like [`push_visible`], but `replacement` is painted in place of `raw`
/// instead of `raw`'s own text — used only for a list marker's bullet/
/// number glyph. `raw` must be non-empty (callers only ever pass an actual
/// marker's byte range).
fn push_replaced(
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
    raw: Range<usize>,
    replacement: &str,
    style: InlineStyle,
) {
    let display_start = display_text.len();
    display_text.push_str(replacement);
    let display_end = display_text.len();
    segments.push(DisplaySegment {
        raw,
        display: display_start..display_end,
        style,
        replaced: true,
    });
}

/// Builds a callout header line's Live Preview rendering — the `> [!type]`
/// (plus optional fold marker/custom title) line itself, not its body
/// (which renders as ordinary blockquote content via
/// `build_display_line_in_context`, unchanged — see `callout.rs`'s own
/// module doc for why only the header needs this). Raw/unstyled if
/// `active`, same as every other construct's active line; otherwise hides
/// `> [!type]+/-` (and one separating space) entirely, then either
/// inline-scans a custom title (so it can itself contain `**bold**`/links/
/// etc., styled [`InlineStyle::CalloutTitle`]) or, if there isn't one,
/// shows the raw type name as typed — capitalized, matching Obsidian's own
/// behavior — as synthetic replacement text in the same style. Falls back
/// to plain `build_display_line` if `line` doesn't actually start with a
/// blockquote prefix or a callout header after it (shouldn't happen for a
/// line `callout::classify_callouts` identified as a header, but cheaper
/// and simpler than unwrapping).
pub fn build_callout_header_line(line: &str, active: bool) -> DisplayLine {
    let indent = line_indent(line);

    if active || line.is_empty() {
        return DisplayLine {
            display_text: line.to_string(),
            segments: vec![DisplaySegment {
                raw: 0..line.len(),
                display: 0..line.len(),
                style: InlineStyle::Plain,
                replaced: false,
            }],
            indent,
        };
    }

    let Some((_, prefix_len)) = blockquote_prefix(line) else {
        return build_display_line(line, active);
    };
    let content = &line[prefix_len..];
    let Some(header) = callout_header_prefix(content) else {
        return build_display_line(line, active);
    };

    let mut segments = Vec::new();
    let mut display_text = String::new();

    let icon = callout::callout_icon(header.kind);

    match header.title {
        Some(title_range) => {
            let title_start = prefix_len + title_range.start;
            push_hidden(&mut segments, &display_text, 0..title_start);
            // A zero-width raw insertion point for the icon prefix — there's
            // no raw byte "spent" on it, same as how `scan_inline` has
            // nothing in `raw` to spend on markup it hides either, just the
            // reverse (showing synthetic text with no raw backing instead of
            // hiding real text with no display backing). `push_replaced`
            // doesn't require a non-empty `raw` in practice, and both
            // `DisplayLine` offset-mapping functions already treat any
            // `is_replaced()` segment as a single collapse point regardless
            // of its raw width, so a click landing on the icon simply
            // resolves to the title's own start, right after it.
            push_replaced(
                &mut segments,
                &mut display_text,
                title_start..title_start,
                &format!("{icon} "),
                InlineStyle::CalloutTitle,
            );
            scan_inline(
                line,
                title_start,
                line.len(),
                InlineStyle::CalloutTitle,
                &mut segments,
                &mut display_text,
            );
        }
        None => {
            let marker_end = prefix_len + header.marker_end;
            push_hidden(&mut segments, &display_text, 0..marker_end);
            let type_name = &content[header.type_name.clone()];
            let fallback_title = format!("{icon} {}", capitalize(type_name));
            push_replaced(
                &mut segments,
                &mut display_text,
                marker_end..line.len(),
                &fallback_title,
                InlineStyle::CalloutTitle,
            );
        }
    }

    DisplayLine {
        display_text,
        segments,
        indent,
    }
}

/// Capitalizes just the first character of `text`, leaving the rest as-is
/// — used for a callout's fallback title (the raw type name as typed,
/// e.g. `important` → `Important`), matching Obsidian's own display.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A line whose entire raw text is hidden (zero display width) — used by
/// `element.rs` for a fenced code block's own delimiter line (e.g.
/// `` ```rust `` or `` ``` ``) when it isn't the active line, and for a
/// table's delimiter row (e.g. `| --- | --- |`), which has no cell content
/// of its own to show at all (`element.rs` paints the header/body divider
/// as a real vector line over it instead of a dashed-rule glyph). Unlike
/// every other hidden-prefix case in this module, there's no remainder to
/// scan: once the fence marker (or the whole delimiter row) is hidden
/// there's nothing left on that line. Always `LineIndent::None` — neither
/// line is ever a list/blockquote line.
pub fn hidden_line(line: &str) -> DisplayLine {
    DisplayLine {
        display_text: String::new(),
        segments: vec![DisplaySegment {
            raw: 0..line.len(),
            display: 0..0,
            style: InlineStyle::Plain,
            replaced: false,
        }],
        indent: LineIndent::None,
    }
}

/// A blockquote prefix's nesting depth (1 for `>`, 2 for `>>` or `> >`, ...)
/// and the byte length of the prefix itself (each `>` plus at most one
/// space right after it), if `line` starts with one.
fn blockquote_prefix(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut depth: u8 = 0;
    while bytes.get(i) == Some(&b'>') {
        i += 1;
        depth += 1;
        if bytes.get(i) == Some(&b' ') {
            i += 1;
        }
    }
    if depth == 0 { None } else { Some((depth, i)) }
}

/// A list marker at the start of a line — just enough to know how many raw
/// bytes to hide and what to replace them with.
enum ListMarker {
    /// `-`, `*`, or `+` followed by a space.
    Unordered,
    /// One to nine digits followed by `.` or `)` and a space; `number` is
    /// shown as-is in the replacement (`"5. "`), not renumbered.
    Ordered { number: u32 },
}

/// `line`'s list marker and its raw byte length, if it starts with one.
/// Deliberately simplified like `heading_prefix`: no leading-space
/// allowance (CommonMark permits up to 3) and exactly one trailing space
/// required (CommonMark allows more, or a tab) — real lists typically
/// satisfy both anyway.
fn list_marker(line: &str) -> Option<(ListMarker, usize)> {
    let bytes = line.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        return Some((ListMarker::Unordered, 2));
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    if !matches!(bytes.get(digits), Some(b'.' | b')')) || bytes.get(digits + 1) != Some(&b' ') {
        return None;
    }
    let number: u32 = line[..digits].parse().ok()?;
    Some((ListMarker::Ordered { number }, digits + 2))
}

/// [`list_marker`], tolerant of leading space-only indentation before the
/// marker — returns the marker, the byte count of that indentation, and the
/// marker's own byte length (not including the indentation). Needed because
/// a marker's nesting *depth* depends on whole-document context
/// ([`classify_block_context`]), not on this single line alone — this
/// function only measures how much indentation is there; it doesn't
/// interpret it. Tabs aren't handled, consistent with this module's other
/// leading-whitespace simplifications (see `heading_prefix`'s own doc
/// comment).
fn indented_list_marker(line: &str) -> Option<(ListMarker, usize, usize)> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    let (marker, marker_len) = list_marker(&line[indent..])?;
    Some((marker, indent, marker_len))
}

/// Whether `line[offset..]` starts with a GFM task-list checkbox
/// (`[ ] `/`[x] `/`[X] `) — `Some(checked)` if so, else `None`. Pure
/// detector; [`push_list_marker`] does the actual rendering decision.
fn task_checkbox_prefix(line: &str, offset: usize) -> Option<bool> {
    let bytes = line.as_bytes();
    match (
        bytes.get(offset),
        bytes.get(offset + 1),
        bytes.get(offset + 2),
        bytes.get(offset + 3),
    ) {
        (Some(b'['), Some(b' '), Some(b']'), Some(b' ')) => Some(false),
        (Some(b'['), Some(b'x' | b'X'), Some(b']'), Some(b' ')) => Some(true),
        _ => None,
    }
}

/// Pushes a list item's marker, folding in a GFM task-list checkbox
/// ([`task_checkbox_prefix`]) right after it if present — returns the
/// offset inline scanning should resume from (past the marker, and the
/// checkbox if there was one).
///
/// An **unordered** marker with a checkbox collapses to a *single*
/// replacement covering both (`"☐ "`/`"☑ "` stands in for the whole
/// `- [ ] `, not `"• " + "☐ "`) — matching Obsidian/GitHub's own task-list
/// rendering, where the checkbox *is* the marker, there's no separate
/// bullet dot alongside it. An **ordered** marker keeps its own number
/// (`"1. "`) and gets the checkbox glyph appended right after
/// (`"1. ☐ "`) — the number still carries meaning a checkbox alone
/// doesn't, so GFM task items on an ordered list keep it.
///
/// Render-only: nothing wires a click on the glyph back to toggling/
/// persisting checked state (deliberately out of scope for this pass, same
/// category as deferred WYSIWYG table editing).
fn push_list_marker(
    line: &str,
    marker_start: usize,
    marker: &ListMarker,
    marker_len: usize,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
) -> usize {
    let marker_end = marker_start + marker_len;
    let checkbox = task_checkbox_prefix(line, marker_end);

    if let (ListMarker::Unordered, Some(checked)) = (marker, checkbox) {
        let glyph = if checked { "☑ " } else { "☐ " };
        push_replaced(
            segments,
            display_text,
            marker_start..marker_end + 4,
            glyph,
            InlineStyle::ListMarker,
        );
        return marker_end + 4;
    }

    let replacement = match marker {
        ListMarker::Unordered => "• ".to_string(),
        ListMarker::Ordered { number } => format!("{number}. "),
    };
    push_replaced(
        segments,
        display_text,
        marker_start..marker_end,
        &replacement,
        InlineStyle::ListMarker,
    );

    match checkbox {
        Some(checked) => {
            let glyph = if checked { "☑ " } else { "☐ " };
            push_replaced(
                segments,
                display_text,
                marker_end..marker_end + 4,
                glyph,
                InlineStyle::ListMarker,
            );
            marker_end + 4
        }
        None => marker_end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The raw ranges Live Preview paints as a link on an inactive line.
    fn rendered_link_texts(line: &str) -> Vec<Range<usize>> {
        build_display_line(line, false)
            .segments
            .iter()
            .filter(|s| matches!(s.style, InlineStyle::Link | InlineStyle::WikiLink))
            .filter(|s| !s.is_hidden())
            .map(|s| s.raw.clone())
            .collect()
    }

    fn scanned_link_texts(line: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        jade_core::link_scan::scan_line(line, |l| {
            if !l.text.is_empty() {
                out.push(l.text);
            }
        });
        out
    }

    /// Rendering and `link_scan` must agree on what is a link, with one known
    /// gap: Live Preview does not scan inside bold, italic or strikethrough
    /// (`**[[a]]**` paints `[[a]]` as plain bold text), while the scanner,
    /// like the backlink index, still counts it. They share the leaf
    /// parsers; this guards the loops around them.
    fn assert_scanner_matches_rendering(line: &str) {
        let rendered = rendered_link_texts(line);
        let display = build_display_line(line, false);
        let scanned = scanned_link_texts(line);
        for r in &rendered {
            assert!(scanned.contains(r), "rendered link not scanned: {line:?}");
        }
        for r in scanned.iter().filter(|r| !rendered.contains(r)) {
            let inside_emphasis = display.segments.iter().any(|s| {
                matches!(
                    s.style,
                    InlineStyle::Bold | InlineStyle::Italic | InlineStyle::Strikethrough
                ) && s.raw.start <= r.start
                    && r.end <= s.raw.end
            });
            assert!(inside_emphasis, "scanned link not rendered: {line:?} {r:?}");
        }
    }

    #[test]
    fn link_scan_agrees_with_rendering_on_hand_written_lines() {
        for line in [
            "plain [[a]] and [[b|c]] and [[d#e]] end",
            "**[[bold link]]** and *[t](u)* and ~~<https://x.y>~~",
            "`[[code]]` then [[real]]",
            "\\[[escaped]] [[after]]",
            "![img](u.png) [text](u.md)",
            "see https://a.b/c. and <me@x.y> and <div>",
            "[a](b) [c](d \"title\") [e](<f g>)",
            "a [[unclosed and [t](unclosed",
            "[[a]] [[b]] [[c]]",
            "# [[heading link]]",
            "> quoted [[link]]",
            "- item [t](u)",
            "[[ ]] [[]] [[a|]] [[#h]]",
            "[x](y)(z) [[a]](b)",
            "é [[Ünï|ä]] ✓ [日本](ü.md)",
        ] {
            assert_scanner_matches_rendering(line);
        }
    }

    #[test]
    fn link_scan_agrees_with_rendering_on_random_inline_soup() {
        let tokens = [
            "[[",
            "]]",
            "[",
            "]",
            "(",
            ")",
            "<",
            ">",
            "`",
            "\\",
            "|",
            "#",
            "**",
            "*",
            "_",
            "~~",
            "https://x.y",
            "a",
            "b c",
            " ",
            "!",
            "é",
            "&amp;",
            "[[a]]",
            "[t](u)",
            "<https://z>",
        ];
        let mut state = 0x9e37_79b9_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as usize
        };
        for _ in 0..20_000 {
            let line: String = (0..next() % 12)
                .map(|_| tokens[next() % tokens.len()])
                .collect();
            assert_scanner_matches_rendering(&line);
        }
    }

    #[test]
    fn link_scan_agrees_with_rendering_on_the_stress_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/benches/fixtures/large_markdown_stress_test.md"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for line in text.lines().take(20_000) {
            assert_scanner_matches_rendering(line);
        }
    }

    fn visible_styles(dl: &DisplayLine) -> Vec<(String, InlineStyle)> {
        dl.segments
            .iter()
            .filter(|s| !s.is_hidden())
            .map(|s| (dl.display_text[s.display.clone()].to_string(), s.style))
            .collect()
    }

    fn assert_contiguous(dl: &DisplayLine, raw_len: usize) {
        let mut expected_raw = 0;
        let mut expected_display = 0;
        for seg in &dl.segments {
            assert_eq!(
                seg.raw.start, expected_raw,
                "raw gap/overlap in {:?}",
                dl.segments
            );
            assert_eq!(
                seg.display.start, expected_display,
                "display gap/overlap in {:?}",
                dl.segments
            );
            expected_raw = seg.raw.end;
            expected_display = seg.display.end;
        }
        assert_eq!(expected_raw, raw_len);
        assert_eq!(expected_display, dl.display_text.len());
    }

    #[test]
    fn active_line_is_shown_verbatim_and_unstyled() {
        let line = "# **bold** [[Link]]";
        let dl = build_display_line(line, true);
        assert_eq!(dl.display_text, line);
        assert_eq!(
            dl.segments,
            vec![DisplaySegment {
                raw: 0..line.len(),
                display: 0..line.len(),
                style: InlineStyle::Plain,
                replaced: false
            }]
        );
    }

    #[test]
    fn empty_line_is_a_single_empty_segment() {
        let dl = build_display_line("", false);
        assert_eq!(dl.display_text, "");
        assert_eq!(
            dl.segments,
            vec![DisplaySegment {
                raw: 0..0,
                display: 0..0,
                style: InlineStyle::Plain,
                replaced: false
            }]
        );
    }

    #[test]
    fn plain_line_with_no_constructs_is_unchanged() {
        let dl = build_display_line("just plain text", false);
        assert_eq!(dl.display_text, "just plain text");
        assert_contiguous(&dl, "just plain text".len());
        assert_eq!(dl.indent, LineIndent::None);
    }

    #[test]
    fn blockquote_indent_matches_its_nesting_depth() {
        assert_eq!(
            build_display_line("> one level", false).indent,
            LineIndent::Blockquote(1)
        );
        assert_eq!(
            build_display_line(">> two levels", false).indent,
            LineIndent::Blockquote(2)
        );
        assert_eq!(
            build_display_line("> > also two levels", false).indent,
            LineIndent::Blockquote(2)
        );
    }

    #[test]
    fn list_item_indent_is_always_one_level() {
        assert_eq!(
            build_display_line("- a dash item", false).indent,
            LineIndent::List(1)
        );
        assert_eq!(
            build_display_line("12. an ordered item", false).indent,
            LineIndent::List(1)
        );
    }

    #[test]
    fn indent_applies_even_on_the_active_line() {
        // The active line renders raw/unstyled (see
        // `active_line_is_shown_verbatim_and_unstyled`), but indent is a
        // structural property, not a Live Preview hiding choice — it must
        // still be set so `element.rs` keeps shifting the line right while
        // it's being edited.
        assert_eq!(
            build_display_line("> quoted", true).indent,
            LineIndent::Blockquote(1)
        );
        assert_eq!(
            build_display_line("- item", true).indent,
            LineIndent::List(1)
        );
    }

    #[test]
    fn line_indent_level_matches_blockquote_depth_or_one_for_a_list() {
        assert_eq!(LineIndent::None.level(), 0);
        assert_eq!(LineIndent::Blockquote(3).level(), 3);
        assert_eq!(LineIndent::List(1).level(), 1);
    }

    #[test]
    fn heading_hides_its_marker_and_styles_the_rest() {
        let line = "## Section Title";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "Section Title");
        assert_eq!(
            visible_styles(&dl),
            vec![("Section Title".to_string(), InlineStyle::Heading(2))]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn heading_without_a_following_space_is_not_a_heading() {
        let line = "##nospace";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "##nospace");
    }

    #[test]
    fn bold_hides_its_delimiters_and_styles_the_content() {
        let line = "a **bold** word";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a bold word");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("a ".to_string(), InlineStyle::Plain),
                ("bold".to_string(), InlineStyle::Bold),
                (" word".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn underscore_bold_is_also_recognized() {
        let dl = build_display_line("__bold__", false);
        assert_eq!(dl.display_text, "bold");
        assert_eq!(
            visible_styles(&dl),
            vec![("bold".to_string(), InlineStyle::Bold)]
        );
    }

    #[test]
    fn single_asterisk_is_italic() {
        let line = "a *italic* word";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a italic word");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("a ".to_string(), InlineStyle::Plain),
                ("italic".to_string(), InlineStyle::Italic),
                (" word".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn strikethrough_hides_its_delimiters_and_styles_the_content() {
        let line = "a ~~gone~~ word";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a gone word");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("a ".to_string(), InlineStyle::Plain),
                ("gone".to_string(), InlineStyle::Strikethrough),
                (" word".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn a_single_tilde_is_not_strikethrough() {
        let line = "a ~lone~ tilde";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn unterminated_strikethrough_falls_back_to_plain_text() {
        let line = "this ~~never closes";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn inline_code_hides_its_backticks() {
        let line = "run `cargo test` now";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "run cargo test now");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("run ".to_string(), InlineStyle::Plain),
                ("cargo test".to_string(), InlineStyle::Code),
                (" now".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn wikilink_without_alias_shows_the_target_and_hides_the_brackets() {
        let line = "see [[Other Note]] please";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "see Other Note please");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("see ".to_string(), InlineStyle::Plain),
                ("Other Note".to_string(), InlineStyle::WikiLink),
                (" please".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn wikilink_with_alias_shows_only_the_alias() {
        let line = "[[Other Note|here]]";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "here");
        assert_eq!(
            visible_styles(&dl),
            vec![("here".to_string(), InlineStyle::WikiLink)]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn wikilink_with_heading_hides_the_heading_suffix() {
        let line = "[[Note#Section]]";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "Note");
        assert_eq!(
            visible_styles(&dl),
            vec![("Note".to_string(), InlineStyle::WikiLink)]
        );
    }

    #[test]
    fn unterminated_bold_falls_back_to_plain_text() {
        let line = "this **never closes";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn unterminated_wikilink_falls_back_to_plain_text() {
        let line = "this [[never closes";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn heading_content_can_itself_contain_inline_constructs() {
        let line = "# a **bold** title";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a bold title");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("a ".to_string(), InlineStyle::Heading(1)),
                ("bold".to_string(), InlineStyle::Bold),
                (" title".to_string(), InlineStyle::Heading(1)),
            ]
        );
    }

    #[test]
    fn display_and_raw_offsets_round_trip_around_a_bold_span() {
        let line = "a **bold** word"; // raw: a(1) **(2) bold(4) **(2) ' word'(5)
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a bold word");

        // Start of "bold" in raw (byte 4, right after "a **") maps to
        // display byte 2 (right after "a ").
        assert_eq!(dl.display_offset_for_raw(4), 2);
        assert_eq!(dl.raw_offset_for_display(2), 4);

        // A raw offset inside the hidden leading "**" collapses to the same
        // display point as the text immediately following it.
        assert_eq!(dl.display_offset_for_raw(2), 2);
        assert_eq!(dl.display_offset_for_raw(3), 2);

        // End of line round-trips too.
        assert_eq!(dl.display_offset_for_raw(line.len()), dl.display_text.len());
        assert_eq!(dl.raw_offset_for_display(dl.display_text.len()), line.len());
    }

    #[test]
    fn raw_and_display_offsets_round_trip_on_an_entirely_hidden_line() {
        let dl = build_display_line("# ", false);
        assert_eq!(dl.display_text, "");
        assert_eq!(dl.raw_offset_for_display(0), 2);
        assert_eq!(dl.display_offset_for_raw(0), 0);
        assert_eq!(dl.display_offset_for_raw(1), 0);
        assert_eq!(dl.display_offset_for_raw(2), 0);
    }

    #[test]
    fn blockquote_hides_its_prefix_and_styles_the_rest() {
        let line = "> a quoted line";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a quoted line");
        assert_eq!(
            visible_styles(&dl),
            vec![("a quoted line".to_string(), InlineStyle::Blockquote(1))]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn nested_blockquote_prefixes_increase_depth() {
        let dl = build_display_line(">> deeper", false);
        assert_eq!(dl.display_text, "deeper");
        assert_eq!(
            visible_styles(&dl),
            vec![("deeper".to_string(), InlineStyle::Blockquote(2))]
        );

        let dl = build_display_line("> > also deeper", false);
        assert_eq!(dl.display_text, "also deeper");
        assert_eq!(
            visible_styles(&dl),
            vec![("also deeper".to_string(), InlineStyle::Blockquote(2))]
        );
    }

    #[test]
    fn blockquote_content_can_itself_contain_inline_constructs() {
        let line = "> a **bold** quote";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a bold quote");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("a ".to_string(), InlineStyle::Blockquote(1)),
                ("bold".to_string(), InlineStyle::Bold),
                (" quote".to_string(), InlineStyle::Blockquote(1)),
            ]
        );
    }

    #[test]
    fn a_heading_inside_a_blockquote_hides_both_prefixes() {
        let line = "> # Quoted heading";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "Quoted heading");
        assert_eq!(
            visible_styles(&dl),
            vec![("Quoted heading".to_string(), InlineStyle::Heading(1))]
        );
        assert_contiguous(&dl, line.len());
        assert_eq!(
            dl.indent,
            LineIndent::Blockquote(1),
            "the line should still indent as a blockquote"
        );
    }

    #[test]
    fn a_list_item_inside_a_blockquote_hides_both_markers() {
        let line = "> - a quoted item";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• a quoted item");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("• ".to_string(), InlineStyle::ListMarker),
                ("a quoted item".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
        assert_eq!(dl.indent, LineIndent::Blockquote(1));
    }

    #[test]
    fn a_nested_blockquote_does_not_also_trigger_list_compounding() {
        // ">>" is already one `blockquote_prefix` match at depth 2 (see
        // `line_indent`'s own doc comment) — `compound_inner` must not also
        // go looking for a list marker right after it.
        let line = ">> deeper text";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "deeper text");
        assert_eq!(
            visible_styles(&dl),
            vec![("deeper text".to_string(), InlineStyle::Blockquote(2))]
        );
    }

    #[test]
    fn a_bare_greater_than_without_following_text_is_still_a_blockquote() {
        let dl = build_display_line(">", false);
        assert_eq!(dl.display_text, "");
        assert_contiguous(&dl, 1);
    }

    #[test]
    fn unordered_list_marker_is_replaced_with_a_bullet() {
        for marker in ["-", "*", "+"] {
            let line = format!("{marker} first item");
            let dl = build_display_line(&line, false);
            assert_eq!(dl.display_text, "• first item", "marker {marker:?}");
            assert_eq!(
                visible_styles(&dl),
                vec![
                    ("• ".to_string(), InlineStyle::ListMarker),
                    ("first item".to_string(), InlineStyle::Plain)
                ],
                "marker {marker:?}"
            );
        }
    }

    #[test]
    fn ordered_list_marker_is_replaced_with_its_canonical_form() {
        let line = "12) ninth item";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "12. ninth item");
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn list_item_content_can_itself_contain_inline_constructs() {
        let line = "- a [[Link]] here";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• a Link here");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("• ".to_string(), InlineStyle::ListMarker),
                ("a ".to_string(), InlineStyle::Plain),
                ("Link".to_string(), InlineStyle::WikiLink),
                (" here".to_string(), InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn a_heading_inside_a_list_item_hides_both_markers() {
        let line = "- # Heading item";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• Heading item");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("• ".to_string(), InlineStyle::ListMarker),
                ("Heading item".to_string(), InlineStyle::Heading(1))
            ]
        );
        assert_contiguous(&dl, line.len());
        assert_eq!(dl.indent, LineIndent::List(1));
    }

    #[test]
    fn a_blockquote_inside_a_list_item_hides_both_markers() {
        let line = "- > quoted item";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• quoted item");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("• ".to_string(), InlineStyle::ListMarker),
                ("quoted item".to_string(), InlineStyle::Blockquote(1))
            ]
        );
        assert_contiguous(&dl, line.len());
        assert_eq!(
            dl.indent,
            LineIndent::List(1),
            "list indent wins over the inner (compounded, same-line) blockquote's"
        );
    }

    #[test]
    fn a_second_dash_inside_a_list_item_is_not_treated_as_a_nested_list() {
        // Nested-list recognition is indentation-based (see
        // `classify_block_context`), not triggered by a second marker
        // appearing right after the first on the very same line — per
        // CommonMark itself, an unindented "- " immediately following
        // another is just plain text starting with "-".
        let line = "- - not nested";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• - not nested");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("• ".to_string(), InlineStyle::ListMarker),
                ("- not nested".to_string(), InlineStyle::Plain)
            ]
        );
    }

    #[test]
    fn a_digit_run_without_a_following_space_is_not_a_list_marker() {
        let dl = build_display_line("12.5 is not a list", false);
        assert_eq!(dl.display_text, "12.5 is not a list");
    }

    #[test]
    fn indented_list_marker_reports_its_leading_whitespace_separately() {
        let (marker, indent, marker_len) = indented_list_marker("   - nested item").unwrap();
        assert!(matches!(marker, ListMarker::Unordered));
        assert_eq!(indent, 3);
        assert_eq!(marker_len, 2);
    }

    #[test]
    fn an_indented_marker_still_hides_its_whitespace_and_marker_in_isolation() {
        // `build_display_line` alone (no document context) can't know this
        // is actually nested — it resolves to depth 1 regardless, same as
        // an unindented marker (see `build_display_line_in_context` for the
        // version that *does* know).
        let line = "  - indented item";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "• indented item");
        assert_eq!(dl.indent, LineIndent::List(1));
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn classify_block_context_resolves_nested_list_depth_by_indentation() {
        let source = "1. outer\n   - inner\n   - inner sibling\n2. outer sibling";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::ListMarkerLine(2),
                LineContext::ListMarkerLine(2),
                LineContext::ListMarkerLine(1),
            ]
        );
    }

    #[test]
    fn classify_block_context_supports_three_levels_of_nesting() {
        let source = "- one\n  - two\n    - three";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::ListMarkerLine(2),
                LineContext::ListMarkerLine(3)
            ]
        );
    }

    #[test]
    fn classify_block_context_a_marker_at_the_same_column_is_a_sibling_not_a_dedent() {
        // The third line's indentation (2 spaces) matches the *second*
        // level's own column exactly — it's a sibling item at that same
        // depth, not a dedent back to the first level.
        let source = "- one\n  - two\n  - two's sibling";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::ListMarkerLine(2),
                LineContext::ListMarkerLine(2)
            ]
        );
    }

    #[test]
    fn list_node_chains_with_the_same_indents_compare_equal_even_when_separately_allocated() {
        // Two independently-built chains representing the same logical
        // nesting (0, then 2) must compare equal regardless of being
        // different `Rc` allocations — `document_cache.rs`'s reconvergence
        // check relies on exactly this (see `ListNode`'s own doc comment):
        // a freshly-walked chain reconverging with a cached one needs
        // value equality, not pointer identity, the same way two
        // `Vec<usize>`s with identical contents already compared equal.
        let a = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2);
        let b = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2);
        assert!(
            !Rc::ptr_eq(&a, &b),
            "sanity check: these really are two separate allocations"
        );
        assert_eq!(a, b);
        assert_eq!(OpenBlock::List(a), OpenBlock::List(b));

        let c = ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 3); // different inner indent
        assert_ne!(
            ListNode::resume_or_push(Some(ListNode::resume_or_push(None, 0)), 2),
            c
        );
    }

    #[test]
    fn classify_block_context_pops_a_level_whose_indentation_is_no_longer_matched() {
        let source = "- one\n  - two\n- one sibling (back to depth 1)";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::ListMarkerLine(2),
                LineContext::ListMarkerLine(1),
            ]
        );
    }

    #[test]
    fn classify_block_context_resolves_lazy_continuation_of_a_blockquote() {
        let source = "> first line\nsecond line, no \">\" of its own";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::Plain,
                LineContext::LazyContinuation(LineIndent::Blockquote(1))
            ]
        );
    }

    #[test]
    fn classify_block_context_resolves_lazy_continuation_of_a_list_item() {
        let source = "- first line\nsecond line, no marker, not even indented";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::LazyContinuation(LineIndent::List(1))
            ]
        );
    }

    #[test]
    fn classify_block_context_multiple_consecutive_lazy_lines_all_continue() {
        let source = "> quote\nsecond\nthird";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::Plain,
                LineContext::LazyContinuation(LineIndent::Blockquote(1)),
                LineContext::LazyContinuation(LineIndent::Blockquote(1)),
            ]
        );
    }

    #[test]
    fn classify_block_context_a_blank_line_ends_lazy_continuation() {
        let source = "> quote\n\nnot part of the quote";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![LineContext::Plain, LineContext::Plain, LineContext::Plain]
        );
    }

    #[test]
    fn classify_block_context_a_heading_ends_an_open_list_or_quote() {
        let source = "- item\n# Heading\nnot part of the list";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::Plain,
                LineContext::Plain
            ]
        );
    }

    #[test]
    fn classify_block_context_a_fresh_blockquote_resets_an_open_list() {
        let source = "- item\n> quote\ncontinues the quote, not the list";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![
                LineContext::ListMarkerLine(1),
                LineContext::Plain,
                LineContext::LazyContinuation(LineIndent::Blockquote(1))
            ]
        );
    }

    #[test]
    fn build_display_line_in_context_overrides_depth_for_a_nested_marker() {
        let line = "  - nested item";
        let dl = build_display_line_in_context(line, false, LineContext::ListMarkerLine(2));
        assert_eq!(dl.display_text, "• nested item");
        assert_eq!(dl.indent, LineIndent::List(2));
    }

    #[test]
    fn build_display_line_in_context_styles_a_lazy_blockquote_continuation() {
        let line = "no marker here, but **bold** still works";
        let dl = build_display_line_in_context(
            line,
            false,
            LineContext::LazyContinuation(LineIndent::Blockquote(2)),
        );
        assert_eq!(dl.display_text, "no marker here, but bold still works");
        assert_eq!(dl.indent, LineIndent::Blockquote(2));
        assert_eq!(
            visible_styles(&dl),
            vec![
                (
                    "no marker here, but ".to_string(),
                    InlineStyle::Blockquote(2)
                ),
                ("bold".to_string(), InlineStyle::Bold),
                (" still works".to_string(), InlineStyle::Blockquote(2)),
            ]
        );
    }

    #[test]
    fn build_display_line_in_context_styles_a_lazy_list_continuation_as_plain() {
        let line = "continues the list item";
        let dl = build_display_line_in_context(
            line,
            false,
            LineContext::LazyContinuation(LineIndent::List(1)),
        );
        assert_eq!(dl.display_text, line);
        assert_eq!(dl.indent, LineIndent::List(1));
        assert_eq!(
            visible_styles(&dl),
            vec![(line.to_string(), InlineStyle::Plain)]
        );
    }

    #[test]
    fn build_display_line_in_context_shows_a_lazy_continuation_raw_when_active() {
        let line = "being edited right now";
        let dl = build_display_line_in_context(
            line,
            true,
            LineContext::LazyContinuation(LineIndent::Blockquote(1)),
        );
        assert_eq!(dl.display_text, line);
        assert_eq!(
            dl.indent,
            LineIndent::Blockquote(1),
            "indent still applies on the active line, same as everywhere else"
        );
    }

    #[test]
    fn display_and_raw_offsets_round_trip_around_a_replaced_list_marker() {
        let line = "- item"; // raw: "-"(1) " "(1) "item"(4), marker = raw 0..2
        let dl = build_display_line(line, false);
        // "•" is 3 UTF-8 bytes, so "• " (the replacement) is 4 bytes, not 2
        // — the whole point of `is_replaced()` existing separately from
        // `is_hidden()` is that this length mismatch is expected.
        assert_eq!(dl.display_text, "• item");
        assert_eq!(dl.display_text.len(), 8);

        // Raw offsets strictly inside the 2-byte marker collapse to display
        // 0, where the replacement bullet begins.
        assert_eq!(dl.display_offset_for_raw(0), 0);
        assert_eq!(dl.display_offset_for_raw(1), 0);
        // Raw offset 2 (right after the marker, start of "item") lands at
        // display 4 (right after the 4-byte "• "), not display 2 — it must
        // skip the glyph's own width, not land inside it.
        assert_eq!(dl.display_offset_for_raw(2), 4);

        // Clicking anywhere on the bullet glyph's display range (0..4, "• ")
        // lands right at the start of the actual item text, raw byte 2.
        assert_eq!(dl.raw_offset_for_display(0), 2);
        assert_eq!(dl.raw_offset_for_display(3), 2);
    }

    #[test]
    fn build_table_rule_is_entirely_hidden() {
        // The delimiter row has no cell content of its own to show at all
        // — `element.rs` paints the header/body divider as a vector quad
        // over this (otherwise blank) line instead of a dashed-rule glyph.
        let line = "| --- | --- |";
        let dl = hidden_line(line);
        assert_eq!(dl.display_text, "");
        assert_contiguous(&dl, line.len());
        assert_eq!(dl.indent, LineIndent::None);
    }

    #[test]
    fn a_cell_line_styles_inline_markup_and_never_reads_a_block() {
        let dl = build_cell_line("**bold** and `code`");
        assert_eq!(dl.display_text, "bold and code");
        assert_eq!(dl.indent, LineIndent::None);
        assert_contiguous(&dl, "**bold** and `code`".len());
        // A leading `-`, `>` or `#` is text inside a cell, not a list, quote or heading.
        for text in ["- item", "> quote", "# not a heading"] {
            let dl = build_cell_line(text);
            assert_eq!(dl.display_text, text);
            assert_eq!(dl.indent, LineIndent::None);
        }
        assert_eq!(build_cell_line(r"a \| b").display_text, "a | b");
    }

    #[test]
    fn backslash_escapes_a_delimiter_and_hides_the_backslash() {
        let line = r"a \*literal\* asterisk";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "a *literal* asterisk");
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn an_escaped_delimiter_does_not_falsely_close_emphasis() {
        // The real closer is the final `*`, not the escaped `\*` in the
        // middle — `find_closing` must skip past it. The emphasis content
        // itself isn't recursively re-scanned for escapes (no construct's
        // content is, in this module — the same simplification that also
        // keeps bold/italic from compounding with each other), so the
        // backslash stays visible verbatim inside it; that's expected, not
        // a bug.
        let line = r"*italic \* still italic*";
        let dl = build_display_line(line, false);
        assert_eq!(
            visible_styles(&dl),
            vec![(r"italic \* still italic".to_string(), InlineStyle::Italic)]
        );
    }

    #[test]
    fn named_entity_decodes_to_its_character() {
        let line = "Bob &amp; Alice";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "Bob & Alice");
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn numeric_and_hex_entities_decode() {
        assert_eq!(build_display_line("&#39;", false).display_text, "'");
        assert_eq!(build_display_line("&#x27;", false).display_text, "'");
    }

    #[test]
    fn unknown_entity_falls_back_to_plain_text() {
        let line = "a & b &notreal; c";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn thematic_break_is_recognized_in_its_three_forms() {
        assert!(is_thematic_break("---"));
        assert!(is_thematic_break("***"));
        assert!(is_thematic_break("___"));
        assert!(is_thematic_break("- - -"));
    }

    #[test]
    fn thematic_break_does_not_match_a_list_item_or_short_run() {
        assert!(!is_thematic_break("- item"));
        assert!(!is_thematic_break("--"));
        assert!(!is_thematic_break("- -* _"));
    }

    #[test]
    fn inline_link_hides_the_url_and_shows_the_text() {
        let line = "see [github](https://github.com) now";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "see github now");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("see ".to_string(), InlineStyle::Plain),
                ("github".to_string(), InlineStyle::Link),
                (" now".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn unterminated_link_falls_back_to_plain_text() {
        let line = "[no closing paren](https://x";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn image_syntax_is_left_untouched() {
        // Images are explicitly deferred (no bitmap-paint pipeline) — the
        // leading `!` must suppress the link match entirely, not render
        // "alt" as a styled link.
        let line = "![alt](https://example.com/x.png)";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn bracketed_autolink_shows_the_url_and_hides_the_angle_brackets() {
        let line = "see <https://example.com> now";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "see https://example.com now");
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("see ".to_string(), InlineStyle::Plain),
                ("https://example.com".to_string(), InlineStyle::Link),
                (" now".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn bracketed_email_autolink_is_recognized() {
        let dl = build_display_line("<a@b.com>", false);
        assert_eq!(dl.display_text, "a@b.com");
        assert_eq!(
            visible_styles(&dl),
            vec![("a@b.com".to_string(), InlineStyle::Link)]
        );
    }

    #[test]
    fn angle_brackets_around_ordinary_text_are_left_untouched() {
        // Raw HTML (`<div>`) is explicitly declined — this must not be
        // mistaken for an autolink.
        let line = "a <div> tag";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn bare_url_autolinks_without_any_delimiter() {
        let line = "visit https://example.com/page today";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, line);
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("visit ".to_string(), InlineStyle::Plain),
                ("https://example.com/page".to_string(), InlineStyle::Link),
                (" today".to_string(), InlineStyle::Plain),
            ]
        );
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn bare_url_trims_one_trailing_punctuation_character() {
        let line = "see https://example.com.";
        let dl = build_display_line(line, false);
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("see ".to_string(), InlineStyle::Plain),
                ("https://example.com".to_string(), InlineStyle::Link),
                (".".to_string(), InlineStyle::Plain),
            ]
        );
    }

    #[test]
    fn task_checkbox_renders_unchecked_and_checked_glyphs() {
        let unchecked = build_display_line("- [ ] todo", false);
        assert_eq!(unchecked.display_text, "☐ todo");
        let checked = build_display_line("- [x] done", false);
        assert_eq!(checked.display_text, "☑ done");
        let checked_upper = build_display_line("- [X] done", false);
        assert_eq!(checked_upper.display_text, "☑ done");
    }

    #[test]
    fn task_checkbox_works_on_an_ordered_marker_too() {
        let dl = build_display_line("1. [ ] todo", false);
        assert_eq!(dl.display_text, "1. ☐ todo");
    }

    #[test]
    fn task_checkbox_works_inside_a_compounding_blockquote() {
        let dl = build_display_line("> - [ ] todo", false);
        assert_eq!(dl.display_text, "☐ todo");
    }

    #[test]
    fn a_list_item_without_a_checkbox_is_unaffected() {
        let dl = build_display_line("- [not a checkbox] text", false);
        assert_eq!(dl.display_text, "• [not a checkbox] text");
    }

    #[test]
    fn callout_header_active_line_is_raw() {
        let line = "> [!info] Custom Title";
        let dl = build_callout_header_line(line, true);
        assert_eq!(dl.display_text, line);
    }

    #[test]
    fn callout_header_without_a_title_shows_the_capitalized_type_name() {
        let dl = build_callout_header_line("> [!warning]", false);
        assert_eq!(
            dl.display_text,
            format!(
                "{} Warning",
                callout::callout_icon(callout::CalloutType::Warning)
            )
        );
        assert_eq!(
            visible_styles(&dl),
            vec![(dl.display_text.clone(), InlineStyle::CalloutTitle)]
        );
    }

    #[test]
    fn callout_header_with_a_custom_title_shows_it_instead() {
        let dl = build_callout_header_line("> [!info] Custom Title", false);
        assert_eq!(
            dl.display_text,
            format!(
                "{} Custom Title",
                callout::callout_icon(callout::CalloutType::Info)
            )
        );
    }

    #[test]
    fn callout_header_hides_the_fold_marker() {
        let dl = build_callout_header_line("> [!info]+ Title", false);
        assert_eq!(
            dl.display_text,
            format!(
                "{} Title",
                callout::callout_icon(callout::CalloutType::Info)
            )
        );
    }

    #[test]
    fn callout_header_title_can_itself_contain_bold() {
        let dl = build_callout_header_line("> [!info] a **bold** title", false);
        assert_eq!(
            dl.display_text,
            format!(
                "{} a bold title",
                callout::callout_icon(callout::CalloutType::Info)
            )
        );
        assert_eq!(
            visible_styles(&dl),
            vec![
                (
                    format!("{} ", callout::callout_icon(callout::CalloutType::Info)),
                    InlineStyle::CalloutTitle
                ),
                ("a ".to_string(), InlineStyle::CalloutTitle),
                ("bold".to_string(), InlineStyle::Bold),
                (" title".to_string(), InlineStyle::CalloutTitle),
            ]
        );
    }

    #[test]
    fn callout_header_falls_back_to_plain_blockquote_rendering_when_not_actually_a_header() {
        let dl = build_callout_header_line("> just a regular quote", false);
        assert_eq!(dl.display_text, "just a regular quote");
        assert_eq!(
            visible_styles(&dl),
            vec![(
                "just a regular quote".to_string(),
                InlineStyle::Blockquote(1)
            )]
        );
    }
}

#[cfg(test)]
mod is_blank_tests {
    use super::is_blank;

    #[test]
    fn matches_trim_is_empty_for_ascii_and_unicode_whitespace() {
        let lines = [
            "",
            " ",
            "  \t ",
            "\t",
            "\r",
            "x",
            " x",
            "# h",
            ">",
            "-",
            "\u{7f}",
            "\u{a0}",
            "\u{a0}x",
            "\u{3000}",
            "\u{3000} x",
            "\u{85}",
            "\u{2003}\u{2003}",
            "é",
            "日本",
        ];
        for line in lines {
            assert_eq!(is_blank(line), line.trim().is_empty(), "{line:?}");
        }
    }
}

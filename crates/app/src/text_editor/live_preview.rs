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
//! `build_table_row`/`build_table_rule` rendering) — not the full
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

use super::table_preview::{self, Column};

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
    /// Content after a hidden blockquote `>` prefix; carries nesting depth
    /// (1 for `>`, 2 for `>>` or `> >`, ...).
    Blockquote(u8),
    /// The bullet/number glyph standing in for a hidden list marker — see
    /// [`DisplaySegment::is_replaced`]. Not applied to a list item's actual
    /// text content, only to the marker's own replacement glyph.
    ListMarker,
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
    /// needed for a table's wrapped sub-rows (`build_table_row`), where
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
            segments: vec![DisplaySegment { raw: 0..line.len(), display: 0..line.len(), style: InlineStyle::Plain, replaced: false }],
            indent,
        };
    }

    let mut segments = Vec::new();
    let mut display_text = String::new();

    if let Some((level, prefix_len)) = heading_prefix(line) {
        push_hidden(&mut segments, &display_text, 0..prefix_len);
        scan_inline(line, prefix_len, line.len(), InlineStyle::Heading(level), &mut segments, &mut display_text);
        return DisplayLine { display_text, segments, indent };
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
        scan_inline(line, content_start, line.len(), style, &mut segments, &mut display_text);
        return DisplayLine { display_text, segments, indent };
    }

    if let Some((marker, leading_ws, marker_len)) = indented_list_marker(line) {
        push_hidden(&mut segments, &display_text, 0..leading_ws);
        let replacement = match marker {
            ListMarker::Unordered => "• ".to_string(),
            ListMarker::Ordered { number } => format!("{number}. "),
        };
        let marker_end = leading_ws + marker_len;
        push_replaced(&mut segments, &mut display_text, leading_ws..marker_end, &replacement, InlineStyle::ListMarker);
        let (content_start, style) =
            compound_inner(line, marker_end, false, true, InlineStyle::Plain, &mut segments, &mut display_text);
        scan_inline(line, content_start, line.len(), style, &mut segments, &mut display_text);
        return DisplayLine { display_text, segments, indent };
    }

    scan_inline(line, 0, line.len(), InlineStyle::Plain, &mut segments, &mut display_text);
    DisplayLine { display_text, segments, indent }
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
        let replacement = match marker {
            ListMarker::Unordered => "• ".to_string(),
            ListMarker::Ordered { number } => format!("{number}. "),
        };
        push_replaced(segments, display_text, offset..offset + marker_len, &replacement, InlineStyle::ListMarker);
        return (offset + marker_len, InlineStyle::Plain);
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
fn line_indent(line: &str) -> LineIndent {
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
pub fn classify_block_context(source: &str) -> Vec<LineContext> {
    enum Open {
        None,
        Blockquote(u8),
        /// Indentation column of each currently-open list level, outermost
        /// first. Never empty while this variant is current — see the
        /// marker-line branch below, which always leaves at least one entry.
        List(Vec<usize>),
    }

    let mut open = Open::None;
    let mut result = Vec::with_capacity(source.matches('\n').count() + 1);

    for line in source.split('\n') {
        if line.trim().is_empty() {
            open = Open::None;
            result.push(LineContext::Plain);
            continue;
        }

        if heading_prefix(line).is_some() {
            open = Open::None;
            result.push(LineContext::Plain);
            continue;
        }

        if let Some((depth, _)) = blockquote_prefix(line) {
            open = Open::Blockquote(depth);
            result.push(LineContext::Plain);
            continue;
        }

        if let Some((_, indent, _)) = indented_list_marker(line) {
            if !matches!(open, Open::List(_)) {
                open = Open::List(Vec::new());
            }
            let Open::List(stack) = &mut open else { unreachable!() };
            while matches!(stack.last(), Some(&top) if top > indent) {
                stack.pop();
            }
            if stack.last() != Some(&indent) {
                stack.push(indent);
            }
            result.push(LineContext::ListMarkerLine(stack.len() as u8));
            continue;
        }

        result.push(match &open {
            Open::Blockquote(depth) => LineContext::LazyContinuation(LineIndent::Blockquote(*depth)),
            Open::List(stack) if !stack.is_empty() => {
                LineContext::LazyContinuation(LineIndent::List(stack.len() as u8))
            }
            _ => LineContext::Plain,
        });
    }

    result
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
                    segments: vec![DisplaySegment { raw: 0..line.len(), display: 0..line.len(), style: InlineStyle::Plain, replaced: false }],
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
            DisplayLine { display_text, segments, indent }
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

enum ConstructKind {
    Code,
    Strong,
    Emphasis,
    Strikethrough,
    WikiLink,
}

struct Construct {
    /// Byte offset of the opener, relative to the slice `find_construct` was
    /// called on (the caller adds its own base offset back in).
    offset: usize,
    opener_len: usize,
    kind: ConstructKind,
}

/// The next inline construct's opener in `rest`, if any — a backtick, a
/// doubled or single `*`/`_` run, a doubled `~` run, or `[[`. A lone `~` (not
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
            b'`' => return Some(Construct { offset: i, opener_len: 1, kind: ConstructKind::Code }),
            b'*' | b'_' => {
                let c = bytes[i];
                return Some(if bytes.get(i + 1) == Some(&c) {
                    Construct { offset: i, opener_len: 2, kind: ConstructKind::Strong }
                } else {
                    Construct { offset: i, opener_len: 1, kind: ConstructKind::Emphasis }
                });
            }
            b'~' if bytes.get(i + 1) == Some(&b'~') => {
                return Some(Construct { offset: i, opener_len: 2, kind: ConstructKind::Strikethrough });
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                return Some(Construct { offset: i, opener_len: 2, kind: ConstructKind::WikiLink });
            }
            _ => i += 1,
        }
    }
    None
}

/// The absolute byte offset of the first occurrence of `delim` in
/// `line[start..end]`, if any.
fn find_closing(line: &str, start: usize, end: usize, delim: &str) -> Option<usize> {
    line[start..end].find(delim).map(|ix| start + ix)
}

struct WikiLinkMatch {
    /// What to actually show: the alias if there is one, else the target
    /// (a `#heading` suffix, if present, is hidden either way).
    visible: Range<usize>,
    /// Absolute offset right after the closing `]]`.
    close_end: usize,
}

/// Parses a `[[...]]` wikilink starting at `start` (pointing at the first
/// `[`), if `line[start..end]` contains one — mirrors
/// `jade_core::links::extract_wikilinks`' target/heading/alias split, but
/// (needing byte offsets for hit-testing, which that function doesn't carry)
/// is its own hand-rolled scan rather than a reuse of it.
fn parse_wikilink(line: &str, start: usize, end: usize) -> Option<WikiLinkMatch> {
    let inner_start = start + 2;
    let close_rel = line[inner_start..end].find("]]")?;
    let inner_end = inner_start + close_rel;
    let close_end = inner_end + 2;
    let inner = &line[inner_start..inner_end];
    if inner.is_empty() {
        return None;
    }

    let visible = match inner.find('|') {
        Some(pipe_rel) => {
            let alias_start = inner_start + pipe_rel + 1;
            if alias_start >= inner_end {
                return None;
            }
            alias_start..inner_end
        }
        None => {
            let target_end = inner.find('#').map_or(inner_end, |h| inner_start + h);
            if target_end <= inner_start {
                return None;
            }
            inner_start..target_end
        }
    };
    Some(WikiLinkMatch { visible, close_end })
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
        let Some(construct) = find_construct(&line[pos..end]) else { break };
        let construct_start = pos + construct.offset;

        push_visible(line, segments, display_text, plain_start..construct_start, default_style);
        plain_start = construct_start;

        let matched = match construct.kind {
            ConstructKind::Code => find_closing(line, construct_start + 1, end, "`").map(|close| {
                push_hidden(segments, display_text, construct_start..construct_start + 1);
                push_visible(line, segments, display_text, construct_start + 1..close, InlineStyle::Code);
                push_hidden(segments, display_text, close..close + 1);
                close + 1
            }),
            ConstructKind::Strong | ConstructKind::Emphasis => {
                let delim_len = construct.opener_len;
                let delim = &line[construct_start..construct_start + delim_len];
                let style = if matches!(construct.kind, ConstructKind::Strong) { InlineStyle::Bold } else { InlineStyle::Italic };
                find_closing(line, construct_start + delim_len, end, delim).map(|close| {
                    push_hidden(segments, display_text, construct_start..construct_start + delim_len);
                    push_visible(line, segments, display_text, construct_start + delim_len..close, style);
                    push_hidden(segments, display_text, close..close + delim_len);
                    close + delim_len
                })
            }
            ConstructKind::Strikethrough => find_closing(line, construct_start + 2, end, "~~").map(|close| {
                push_hidden(segments, display_text, construct_start..construct_start + 2);
                push_visible(line, segments, display_text, construct_start + 2..close, InlineStyle::Strikethrough);
                push_hidden(segments, display_text, close..close + 2);
                close + 2
            }),
            ConstructKind::WikiLink => parse_wikilink(line, construct_start, end).map(|link| {
                push_hidden(segments, display_text, construct_start..link.visible.start);
                push_visible(line, segments, display_text, link.visible.clone(), InlineStyle::WikiLink);
                push_hidden(segments, display_text, link.visible.end..link.close_end);
                link.close_end
            }),
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
    push_visible(line, segments, display_text, plain_start..end, default_style);
}

fn push_hidden(segments: &mut Vec<DisplaySegment>, display_text: &str, raw: Range<usize>) {
    if raw.is_empty() {
        return;
    }
    let at = display_text.len();
    segments.push(DisplaySegment { raw, display: at..at, style: InlineStyle::Plain, replaced: false });
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
    segments.push(DisplaySegment { raw, display: display_start..display_end, style, replaced: false });
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
    segments.push(DisplaySegment { raw, display: display_start..display_end, style, replaced: true });
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
        segments: vec![DisplaySegment { raw: 0..line.len(), display: 0..0, style: InlineStyle::Plain, replaced: false }],
        indent: LineIndent::None,
    }
}

/// Renders one header or body row of a table as one or more actually-
/// aligned grid lines — unlike every other construct in this module, which
/// only hides or replaces markup *in place*, a table's cells get padded to
/// each column's resolved width and separated by blank space, since the raw
/// pipe-delimited text has no consistent column alignment of its own. The
/// grid lines themselves (vertical column dividers, the outer box, the
/// header/body rule) are *not* drawn here as box-drawing glyphs — a
/// monospace font's glyphs don't reliably abut pixel-for-pixel across
/// separately-shaped/painted lines, which read as visible seams/gaps.
/// `element.rs`'s `paint_lines` instead paints them as real vector quads
/// (`table_preview::column_divider_char_offsets` gives it the exact
/// character columns to convert to pixel x-positions) directly over this
/// function's blank gaps — this function's only job is leaving those gaps
/// the right width and getting cell *content* positioned and aligned.
/// Tractable without any pixel measurement because this editor's font is
/// always monospace (`element.rs`'s `render_editor`) — a character count
/// lines up identically across every row.
///
/// More than one line is returned only when `max_row_width_chars` is given
/// and at least one cell's text doesn't fit its column within that budget
/// — `table_preview::cap_column_widths` shrinks the natural (`columns`)
/// widths to fit first, and any cell still too wide for its now-narrower
/// column wraps onto additional lines *within that cell*
/// (`table_preview::wrap_cell_text`) rather than letting the row overflow
/// the editor's own width. `None` skips capping/wrapping entirely (the
/// natural widths are used verbatim, always exactly one line) — a caller
/// with no width budget to measure against.
///
/// Each cell's own text is pushed via `push_cell_content`, which keeps it a
/// genuinely *visible*, byte-mapped segment (so clicking into it lands
/// inside its actual raw content, unlike a blank gap, which always
/// collapses to whatever follows it when clicked) in the common
/// single-line case — but once a cell's content is split across multiple
/// *returned* lines, no single line's text is the cell's whole verbatim
/// content any more, so every line falls back to `push_replaced` for every
/// cell instead (collapsing to that cell's raw start when clicked, the
/// same "collapses to one anchor point" behavior a blank gap or list
/// marker's bullet already has — not byte-exact within a wrapped cell, a
/// deliberate v1 scope cut). Cells are never inline-scanned for
/// bold/italic/etc. either way (GFM cells are inline-content-only anyway,
/// so this isn't a *compounding* gap the way blockquote/list markup is,
/// just a simplification). A missing cell (this row has fewer cells than
/// `columns`) gets a zero-width raw range at the line's end, same
/// treatment `build_display_line_in_context` gives a lazy continuation's
/// missing prefix. The real leading/trailing pipe byte (if present — both
/// are optional per GFM) is replaced with blank space; if absent, a
/// zero-width insertion stands in for it, same "synthetic, no raw
/// counterpart" pattern — repeated on every returned line, so a wrapped
/// cell still reads as one continuous block.
pub fn build_table_row(line: &str, columns: &[Column], max_row_width_chars: Option<usize>) -> Vec<DisplayLine> {
    let row = table_preview::scan_row(line);
    let widths: Vec<usize> = table_preview::resolve_widths(columns, max_row_width_chars);

    let wrapped_cells: Vec<Vec<String>> = (0..columns.len())
        .map(|i| {
            let text = row.cells.get(i).map_or("", |cell| cell.text.as_str());
            table_preview::wrap_cell_text(text, widths[i])
        })
        .collect();
    let sub_rows = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1).max(1);

    if sub_rows == 1 {
        return vec![build_table_row_line(line, &row, columns, &widths)];
    }

    (0..sub_rows)
        .map(|sub_row| build_table_row_wrapped_line(&row, columns, &widths, &wrapped_cells, sub_row, line.len()))
        .collect()
}

/// The common (non-wrapped) case: exactly one line, every cell's full text
/// via the precise, byte-mapped `push_cell_content`. Split out from
/// [`build_table_row`] so that path (still the overwhelming majority of
/// tables, which fit without any cell needing to wrap) stays byte-exact —
/// only [`build_table_row_wrapped_line`], used when it doesn't, pays for
/// the less precise fallback.
fn build_table_row_line(line: &str, row: &table_preview::Row, columns: &[Column], widths: &[usize]) -> DisplayLine {
    let mut segments = Vec::new();
    let mut display_text = String::new();

    let leading_raw = if row.has_leading_pipe { 0..1 } else { 0..0 };
    push_replaced(&mut segments, &mut display_text, leading_raw, "  ", InlineStyle::Plain);

    let real_count = row.cells.len();
    let mut trailing_pipe_consumed = false;

    for (i, &width) in widths.iter().enumerate() {
        let align = columns[i].align;
        match row.cells.get(i) {
            Some(cell) => push_cell_content(line, &mut segments, &mut display_text, cell, width, align),
            None => {
                let padded = table_preview::pad("", width, align);
                push_replaced(&mut segments, &mut display_text, line.len()..line.len(), &padded, InlineStyle::Plain);
            }
        }

        let is_last_column = i + 1 == widths.len();
        let sep_text = if is_last_column { "  " } else { "   " };
        // The real interior pipe between this cell and the next only
        // exists if both are actually present in the source; the real
        // trailing pipe (if any) is consumed exactly once, whichever
        // column turns out to be the last *real* one — any columns past
        // that are purely synthetic and must not re-claim the same byte.
        let sep_raw = if i + 1 < real_count {
            row.cells[i].raw.end..row.cells[i].raw.end + 1
        } else if !trailing_pipe_consumed && row.has_trailing_pipe {
            trailing_pipe_consumed = true;
            line.len() - 1..line.len()
        } else {
            line.len()..line.len()
        };
        push_replaced(&mut segments, &mut display_text, sep_raw, sep_text, InlineStyle::Plain);
    }

    DisplayLine { display_text, segments, indent: LineIndent::None }
}

/// One sub-row of a row that needed cell-wrapping: for each column, either
/// that cell's `sub_row`-th wrapped line (if it has one) or blank padding
/// (if its wrapped text is shorter than the tallest cell in this row) —
/// every cell uses `push_replaced` with its *whole* raw range regardless
/// (see [`build_table_row`]'s own doc comment for why byte-exact mapping
/// isn't possible here).
fn build_table_row_wrapped_line(
    row: &table_preview::Row,
    columns: &[Column],
    widths: &[usize],
    wrapped_cells: &[Vec<String>],
    sub_row: usize,
    line_len: usize,
) -> DisplayLine {
    let mut segments = Vec::new();
    let mut display_text = String::new();

    let leading_raw = if row.has_leading_pipe { 0..1 } else { 0..0 };
    push_replaced(&mut segments, &mut display_text, leading_raw, "  ", InlineStyle::Plain);

    let real_count = row.cells.len();
    let mut trailing_pipe_consumed = false;

    for (i, &width) in widths.iter().enumerate() {
        let text = wrapped_cells[i].get(sub_row).map_or("", String::as_str);
        let padded = table_preview::pad(text, width, columns[i].align);
        let raw = row.cells.get(i).map_or(line_len..line_len, |cell| cell.raw.clone());
        push_replaced(&mut segments, &mut display_text, raw, &padded, InlineStyle::Plain);

        // Same real-pipe-position logic as `build_table_row_line`, so raw
        // coverage stays contiguous — this sub-row still only claims each
        // byte once, it just maps every cell's *whole* range to one
        // combined blob (its text on this sub-row, or blank) rather than a
        // byte-exact slice.
        let is_last_column = i + 1 == widths.len();
        let sep_text = if is_last_column { "  " } else { "   " };
        let sep_raw = if i + 1 < real_count {
            row.cells[i].raw.end..row.cells[i].raw.end + 1
        } else if !trailing_pipe_consumed && row.has_trailing_pipe {
            trailing_pipe_consumed = true;
            line_len - 1..line_len
        } else {
            line_len..line_len
        };
        push_replaced(&mut segments, &mut display_text, sep_raw, sep_text, InlineStyle::Plain);
    }

    DisplayLine { display_text, segments, indent: LineIndent::None }
}

/// Pushes one cell's padded, alignment-adjusted content as (up to) three
/// segments: the original leading/trailing whitespace `trim()` already
/// removed from `cell.text` (replaced with this column's actual alignment
/// padding — `cell.raw` minus that whitespace is exactly where the real
/// trimmed content lives), and the trimmed text itself. That middle segment
/// is pushed via `push_visible` (a genuine raw↔display byte mapping, not
/// `push_replaced`) whenever `cell.text` has no backslash in it — i.e. no
/// `\|` for `table_preview::scan_row` to have unescaped — so clicking into
/// the cell's own text resolves to a real position inside it rather than
/// falling through to whatever comes after, the way a blank gap or a list
/// marker's bullet always does. The rarer escaped-pipe case falls back to
/// one `push_replaced` segment for the whole cell (display length no
/// longer matches raw length once the escape is collapsed, so a byte-exact
/// mapping isn't possible) — same "collapses to the next segment" click
/// behavior a blank gap already has, not a regression, just not as precise.
fn push_cell_content(
    line: &str,
    segments: &mut Vec<DisplaySegment>,
    display_text: &mut String,
    cell: &table_preview::Cell,
    width: usize,
    align: table_preview::Align,
) {
    let raw_slice = &line[cell.raw.clone()];
    if raw_slice.contains('\\') {
        let padded = table_preview::pad(&cell.text, width, align);
        push_replaced(segments, display_text, cell.raw.clone(), &padded, InlineStyle::Plain);
        return;
    }

    let content_start = cell.raw.start + (raw_slice.len() - raw_slice.trim_start().len());
    let content_end = cell.raw.start + raw_slice.trim_end().len();
    let (left_pad, right_pad) = table_preview::pad_amounts(cell.text.chars().count(), width, align);

    push_replaced(segments, display_text, cell.raw.start..content_start, &" ".repeat(left_pad), InlineStyle::Plain);
    push_visible(line, segments, display_text, content_start..content_end, InlineStyle::Plain);
    push_replaced(segments, display_text, content_end..cell.raw.end, &" ".repeat(right_pad), InlineStyle::Plain);
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

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(seg.raw.start, expected_raw, "raw gap/overlap in {:?}", dl.segments);
            assert_eq!(seg.display.start, expected_display, "display gap/overlap in {:?}", dl.segments);
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
        assert_eq!(dl.segments, vec![DisplaySegment { raw: 0..line.len(), display: 0..line.len(), style: InlineStyle::Plain, replaced: false }]);
    }

    #[test]
    fn empty_line_is_a_single_empty_segment() {
        let dl = build_display_line("", false);
        assert_eq!(dl.display_text, "");
        assert_eq!(dl.segments, vec![DisplaySegment { raw: 0..0, display: 0..0, style: InlineStyle::Plain, replaced: false }]);
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
        assert_eq!(build_display_line("> one level", false).indent, LineIndent::Blockquote(1));
        assert_eq!(build_display_line(">> two levels", false).indent, LineIndent::Blockquote(2));
        assert_eq!(build_display_line("> > also two levels", false).indent, LineIndent::Blockquote(2));
    }

    #[test]
    fn list_item_indent_is_always_one_level() {
        assert_eq!(build_display_line("- a dash item", false).indent, LineIndent::List(1));
        assert_eq!(build_display_line("12. an ordered item", false).indent, LineIndent::List(1));
    }

    #[test]
    fn indent_applies_even_on_the_active_line() {
        // The active line renders raw/unstyled (see
        // `active_line_is_shown_verbatim_and_unstyled`), but indent is a
        // structural property, not a Live Preview hiding choice — it must
        // still be set so `element.rs` keeps shifting the line right while
        // it's being edited.
        assert_eq!(build_display_line("> quoted", true).indent, LineIndent::Blockquote(1));
        assert_eq!(build_display_line("- item", true).indent, LineIndent::List(1));
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
        assert_eq!(visible_styles(&dl), vec![("Section Title".to_string(), InlineStyle::Heading(2))]);
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
        assert_eq!(visible_styles(&dl), vec![("bold".to_string(), InlineStyle::Bold)]);
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
        assert_eq!(visible_styles(&dl), vec![("here".to_string(), InlineStyle::WikiLink)]);
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn wikilink_with_heading_hides_the_heading_suffix() {
        let line = "[[Note#Section]]";
        let dl = build_display_line(line, false);
        assert_eq!(dl.display_text, "Note");
        assert_eq!(visible_styles(&dl), vec![("Note".to_string(), InlineStyle::WikiLink)]);
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
        assert_eq!(visible_styles(&dl), vec![("a quoted line".to_string(), InlineStyle::Blockquote(1))]);
        assert_contiguous(&dl, line.len());
    }

    #[test]
    fn nested_blockquote_prefixes_increase_depth() {
        let dl = build_display_line(">> deeper", false);
        assert_eq!(dl.display_text, "deeper");
        assert_eq!(visible_styles(&dl), vec![("deeper".to_string(), InlineStyle::Blockquote(2))]);

        let dl = build_display_line("> > also deeper", false);
        assert_eq!(dl.display_text, "also deeper");
        assert_eq!(visible_styles(&dl), vec![("also deeper".to_string(), InlineStyle::Blockquote(2))]);
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
        assert_eq!(visible_styles(&dl), vec![("Quoted heading".to_string(), InlineStyle::Heading(1))]);
        assert_contiguous(&dl, line.len());
        assert_eq!(dl.indent, LineIndent::Blockquote(1), "the line should still indent as a blockquote");
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
        assert_eq!(visible_styles(&dl), vec![("deeper text".to_string(), InlineStyle::Blockquote(2))]);
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
                vec![("• ".to_string(), InlineStyle::ListMarker), ("first item".to_string(), InlineStyle::Plain)],
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
            vec![("• ".to_string(), InlineStyle::ListMarker), ("Heading item".to_string(), InlineStyle::Heading(1))]
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
            vec![("• ".to_string(), InlineStyle::ListMarker), ("quoted item".to_string(), InlineStyle::Blockquote(1))]
        );
        assert_contiguous(&dl, line.len());
        assert_eq!(dl.indent, LineIndent::List(1), "list indent wins over the inner (compounded, same-line) blockquote's");
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
            vec![("• ".to_string(), InlineStyle::ListMarker), ("- not nested".to_string(), InlineStyle::Plain)]
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
        assert_eq!(ctx, vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(1),
        ]);
    }

    #[test]
    fn classify_block_context_supports_three_levels_of_nesting() {
        let source = "- one\n  - two\n    - three";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![LineContext::ListMarkerLine(1), LineContext::ListMarkerLine(2), LineContext::ListMarkerLine(3)]
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
            vec![LineContext::ListMarkerLine(1), LineContext::ListMarkerLine(2), LineContext::ListMarkerLine(2)]
        );
    }

    #[test]
    fn classify_block_context_pops_a_level_whose_indentation_is_no_longer_matched() {
        let source = "- one\n  - two\n- one sibling (back to depth 1)";
        let ctx = classify_block_context(source);
        assert_eq!(ctx, vec![
            LineContext::ListMarkerLine(1),
            LineContext::ListMarkerLine(2),
            LineContext::ListMarkerLine(1),
        ]);
    }

    #[test]
    fn classify_block_context_resolves_lazy_continuation_of_a_blockquote() {
        let source = "> first line\nsecond line, no \">\" of its own";
        let ctx = classify_block_context(source);
        assert_eq!(ctx, vec![LineContext::Plain, LineContext::LazyContinuation(LineIndent::Blockquote(1))]);
    }

    #[test]
    fn classify_block_context_resolves_lazy_continuation_of_a_list_item() {
        let source = "- first line\nsecond line, no marker, not even indented";
        let ctx = classify_block_context(source);
        assert_eq!(ctx, vec![LineContext::ListMarkerLine(1), LineContext::LazyContinuation(LineIndent::List(1))]);
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
        assert_eq!(ctx, vec![LineContext::Plain, LineContext::Plain, LineContext::Plain]);
    }

    #[test]
    fn classify_block_context_a_heading_ends_an_open_list_or_quote() {
        let source = "- item\n# Heading\nnot part of the list";
        let ctx = classify_block_context(source);
        assert_eq!(ctx, vec![LineContext::ListMarkerLine(1), LineContext::Plain, LineContext::Plain]);
    }

    #[test]
    fn classify_block_context_a_fresh_blockquote_resets_an_open_list() {
        let source = "- item\n> quote\ncontinues the quote, not the list";
        let ctx = classify_block_context(source);
        assert_eq!(
            ctx,
            vec![LineContext::ListMarkerLine(1), LineContext::Plain, LineContext::LazyContinuation(LineIndent::Blockquote(1))]
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
        let dl = build_display_line_in_context(line, false, LineContext::LazyContinuation(LineIndent::Blockquote(2)));
        assert_eq!(dl.display_text, "no marker here, but bold still works");
        assert_eq!(dl.indent, LineIndent::Blockquote(2));
        assert_eq!(
            visible_styles(&dl),
            vec![
                ("no marker here, but ".to_string(), InlineStyle::Blockquote(2)),
                ("bold".to_string(), InlineStyle::Bold),
                (" still works".to_string(), InlineStyle::Blockquote(2)),
            ]
        );
    }

    #[test]
    fn build_display_line_in_context_styles_a_lazy_list_continuation_as_plain() {
        let line = "continues the list item";
        let dl = build_display_line_in_context(line, false, LineContext::LazyContinuation(LineIndent::List(1)));
        assert_eq!(dl.display_text, line);
        assert_eq!(dl.indent, LineIndent::List(1));
        assert_eq!(visible_styles(&dl), vec![(line.to_string(), InlineStyle::Plain)]);
    }

    #[test]
    fn build_display_line_in_context_shows_a_lazy_continuation_raw_when_active() {
        let line = "being edited right now";
        let dl = build_display_line_in_context(line, true, LineContext::LazyContinuation(LineIndent::Blockquote(1)));
        assert_eq!(dl.display_text, line);
        assert_eq!(dl.indent, LineIndent::Blockquote(1), "indent still applies on the active line, same as everywhere else");
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

    fn two_columns() -> Vec<Column> {
        vec![
            Column { width: 5, align: table_preview::Align::Left },
            Column { width: 3, align: table_preview::Align::Left },
        ]
    }

    #[test]
    fn build_table_row_pads_cells_to_column_width_with_blank_gaps() {
        let line = "| Alice | 30 |";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        assert_eq!(dl.display_text, "  Alice   30   ");
        assert_contiguous(dl, line.len());
        assert_eq!(dl.indent, LineIndent::None);
    }

    #[test]
    fn build_table_row_leaves_every_segment_plain_no_border_glyphs_drawn() {
        // No box-drawing glyphs are drawn as text any more — the grid lines
        // are painted as real vector quads by `element.rs` directly over
        // these blank gaps instead, so there's nothing left here to give a
        // distinct style to.
        let dl = &build_table_row("| Alice | 30 |", &two_columns(), None)[0];
        assert!(visible_styles(dl).iter().all(|(_, style)| *style == InlineStyle::Plain));
    }

    #[test]
    fn build_table_row_right_aligns_per_column_spec() {
        let columns = vec![Column { width: 5, align: table_preview::Align::Right }];
        let dl = &build_table_row("| x |", &columns, None)[0];
        assert_eq!(dl.display_text, "      x  ");
    }

    #[test]
    fn build_table_row_pads_a_missing_cell_as_empty_with_a_zero_width_raw_range() {
        let line = "| Alice |";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        assert_eq!(dl.display_text, "  Alice        ");
        assert_contiguous(dl, line.len());
    }

    #[test]
    fn build_table_row_handles_a_row_with_no_leading_or_trailing_pipe() {
        let line = "Alice | 30";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        assert_eq!(dl.display_text, "  Alice   30   ");
        assert_contiguous(dl, line.len());
    }

    #[test]
    fn build_table_row_click_offsets_round_trip_into_the_right_cell() {
        let line = "| Alice | 30 |";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        // "30" starts right after the blank gap following "Alice" in the
        // display text.
        let thirty_display_start = dl.display_text.find("30").unwrap();
        let raw = dl.raw_offset_for_display(thirty_display_start);
        let thirty_raw_start = line.find("30").unwrap();
        assert_eq!(raw, thirty_raw_start, "clicking on the second cell should land in its raw byte range");
    }

    #[test]
    fn build_table_row_falls_back_to_a_replaced_segment_for_an_escaped_pipe_cell() {
        let line = r"| a \| b | 30 |";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        assert_contiguous(dl, line.len());
        assert!(dl.display_text.contains("a | b"), "the escaped pipe should still unescape to a literal |");
    }

    #[test]
    fn build_table_row_is_contiguous_with_real_alignment_padding() {
        let columns =
            vec![Column { width: 6, align: table_preview::Align::Right }, Column { width: 3, align: table_preview::Align::Center }];
        let line = "|  x  |  y  |";
        let dl = &build_table_row(line, &columns, None)[0];
        assert_contiguous(dl, line.len());
    }

    #[test]
    fn build_table_row_leaves_every_grid_line_position_blank() {
        // `element.rs` paints the actual grid lines as vector quads at
        // these exact character offsets — this function's own job is only
        // to make sure it never puts visible text there.
        let line = "| Alice | 30 |";
        let dl = &build_table_row(line, &two_columns(), None)[0];
        let widths = table_preview::resolve_widths(&two_columns(), None);
        for offset in table_preview::column_divider_char_offsets(&widths) {
            assert_eq!(dl.display_text.chars().nth(offset), Some(' '), "grid line position {offset} should be blank");
        }
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

    fn two_wide_columns() -> Vec<Column> {
        vec![
            // 21 chars — exactly fits "Alexandria Fitzgerald" unwrapped.
            Column { width: 21, align: table_preview::Align::Left },
            Column { width: 3, align: table_preview::Align::Left },
        ]
    }

    #[test]
    fn build_table_row_without_a_budget_never_wraps() {
        let line = "| Alexandria Fitzgerald | 41 |";
        let rows = build_table_row(line, &two_wide_columns(), None);
        assert_eq!(rows.len(), 1, "no width budget at all means no capping/wrapping");
    }

    #[test]
    fn build_table_row_wraps_a_cell_too_wide_for_its_capped_column() {
        let line = "| Alexandria Fitzgerald | 41 |";
        // Border overhead for 2 columns = 7; a budget of 17 leaves only 10
        // content characters, capping column 0 well below its natural
        // (20-character) width, so "Alexandria Fitzgerald" no longer fits
        // on one line.
        let rows = build_table_row(line, &two_wide_columns(), Some(17));
        assert!(rows.len() > 1, "a cell too wide for its capped column should wrap onto more than one sub-row");
        for dl in &rows {
            assert_contiguous(dl, line.len());
            assert_eq!(dl.indent, LineIndent::None);
        }
    }

    #[test]
    fn build_table_row_leaves_a_short_cells_extra_sub_rows_blank() {
        let line = "| Alexandria Fitzgerald | 41 |";
        let rows = build_table_row(line, &two_wide_columns(), Some(17));
        assert!(rows.len() > 1);
        // Column 1's "41" fits on a single wrapped line (width 3) even
        // though column 0 needs several — it should show up on exactly
        // one sub-row, blank-padded on every other one.
        let containing_41 = rows.iter().filter(|dl| dl.display_text.contains("41")).count();
        assert_eq!(containing_41, 1, "a short cell's text should appear on exactly one sub-row");
    }

    #[test]
    fn build_table_row_wrapped_cells_collapse_clicks_to_their_own_raw_start() {
        let line = "| Alexandria Fitzgerald | 41 |";
        let rows = build_table_row(line, &two_wide_columns(), Some(17));
        // The cell's raw range starts right after the leading pipe,
        // including the space before "Alexandria" — not
        // `line.find("Alexandria")` itself, which skips that space.
        let col0_raw_start = table_preview::scan_row(line).cells[0].raw.start;
        for dl in &rows {
            // Clicking anywhere a wrapped cell's own text is shown should
            // land at that cell's raw start — the documented "collapses to
            // one anchor point" simplification `build_table_row`'s own doc
            // comment describes for the wrapped case. Every segment here is
            // `is_replaced()` (no cell's text is byte-exact to its wrapped
            // sub-line), so `nearest_segment_raw_start` is the method that
            // actually applies — not `raw_offset_for_display`, which would
            // skip every one of them and fall through to the line's end.
            if let Some(display_start) = dl.display_text.find("Alexand") {
                assert_eq!(dl.nearest_segment_raw_start(display_start), col0_raw_start);
            }
        }
    }
}

//! Per-line Live Preview rendering model: how a note's *raw* Markdown line
//! maps to what should actually be painted — hidden markup (heading `#`
//! prefixes, `**`/`*` emphasis delimiters, `` ` `` code fences, `[[`/`]]`/
//! `|target` wikilink syntax) collapsed to zero display width, the remaining
//! text tagged with the style it should be painted in.
//!
//! Deliberately GPUI-free (same pure-logic-first approach `buffer.rs` and
//! `markdown_blocks.rs` already use) — `element.rs`/`input_handler.rs`
//! consume [`DisplaySegment::style`] to pick an actual `Font`/color, and its
//! raw↔display byte mapping ([`DisplayLine::display_offset_for_raw`]/
//! [`DisplayLine::raw_offset_for_display`]) to convert between a raw buffer
//! offset and an x position in the *display* text they actually shape and
//! paint — the two are no longer the same string once markup is hidden.
//!
//! Scope deliberately narrow for this first pass: headings, bold, italic,
//! inline code, wikilinks — not the full CommonMark/GFM inline grammar (no
//! strikethrough, tables, autolinks). Emphasis matching is also simplified:
//! `**x**`/`__x__` is bold, a lone `*x*`/`_x_` is italic, without
//! CommonMark's full left/right-flanking delimiter-run algorithm — a
//! deliberate v1 scope cut, in the same spirit as `markdown_blocks.rs`'s own
//! simplifications (a whole list is always one block, etc).
//!
//! The line containing the cursor is never hidden — `build_display_line`'s
//! `active` flag turns rendering off entirely for it, showing the literal
//! raw text unstyled. (A "real" Live Preview editor keeps *formatting*
//! visible while editing and hides only the delimiter characters; doing that
//! too is a reasonable follow-up, not required for this first pass given the
//! narrow construct list above.)

use std::ops::Range;

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
    WikiLink,
}

/// One contiguous run of a line: the byte range it occupies in the *raw*
/// line, the byte range it occupies in [`DisplayLine::display_text`], and
/// the style to paint that display range in. A hidden run of markup (the
/// `**`/`` ` ``/`[[`/... delimiters themselves) has a non-empty `raw` range
/// but an empty `display` range — it contributes nothing to what's painted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySegment {
    pub raw: Range<usize>,
    pub display: Range<usize>,
    pub style: InlineStyle,
}

impl DisplaySegment {
    /// Markup that's collapsed to zero display width — by construction,
    /// every segment this module builds is either hidden with an empty
    /// `display` range, or visible with `display.len() == raw.len()`
    /// (visible text is shown verbatim, never re-encoded), so checking
    /// `display` alone is enough to tell them apart.
    pub fn is_hidden(&self) -> bool {
        self.display.is_empty()
    }
}

/// A line's raw Markdown text alongside what should actually be painted for
/// it: `display_text` (the hidden-markup-free, or unchanged-if-active,
/// string to shape) and `segments` (the raw↔display mapping and per-run
/// style covering it). `segments` are in raw order, contiguous, and cover
/// the whole line (`0..raw_line.len()`) with no gaps or overlaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    pub display_text: String,
    pub segments: Vec<DisplaySegment>,
}

impl DisplayLine {
    /// The display byte offset corresponding to raw byte offset `raw_offset`
    /// (which must be within this line's raw range). A `raw_offset` inside a
    /// hidden segment collapses to that segment's single display point —
    /// there's nothing else to map it to, since hidden markup paints nothing.
    pub fn display_offset_for_raw(&self, raw_offset: usize) -> usize {
        for seg in &self.segments {
            if raw_offset <= seg.raw.end {
                if seg.is_hidden() {
                    return seg.display.start;
                }
                let delta = raw_offset.saturating_sub(seg.raw.start);
                return seg.display.start + delta;
            }
        }
        self.display_text.len()
    }

    /// The raw byte offset corresponding to display byte offset
    /// `display_offset` — the inverse of [`display_offset_for_raw`]. Hidden
    /// segments are skipped: they occupy no display width, so a real
    /// `display_offset` (e.g. from a mouse click against shaped
    /// `display_text`) can never fall inside one.
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
            if seg.is_hidden() {
                continue;
            }
            if display_offset < seg.display.end {
                let delta = display_offset.saturating_sub(seg.display.start);
                return seg.raw.start + delta;
            }
        }
        self.segments.last().map_or(0, |s| s.raw.end)
    }
}

/// Builds the Live Preview rendering of one `\n`-free `line`: unchanged if
/// `active` (the line containing the cursor) or empty, else with headings/
/// bold/italic/inline-code/wikilink markup hidden per this module's doc
/// comment.
pub fn build_display_line(line: &str, active: bool) -> DisplayLine {
    if active || line.is_empty() {
        return DisplayLine {
            display_text: line.to_string(),
            segments: vec![DisplaySegment { raw: 0..line.len(), display: 0..line.len(), style: InlineStyle::Plain }],
        };
    }

    let mut segments = Vec::new();
    let mut display_text = String::new();

    let start = if let Some((level, prefix_len)) = heading_prefix(line) {
        push_hidden(&mut segments, &display_text, 0..prefix_len);
        scan_inline(line, prefix_len, line.len(), InlineStyle::Heading(level), &mut segments, &mut display_text);
        return DisplayLine { display_text, segments };
    } else {
        0
    };

    scan_inline(line, start, line.len(), InlineStyle::Plain, &mut segments, &mut display_text);
    DisplayLine { display_text, segments }
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
/// doubled or single `*`/`_` run, or `[[`. Scans byte-by-byte; safe over
/// multi-byte UTF-8 since none of its continuation bytes can match any of
/// these ASCII patterns, so stepping through one at a time just finds
/// nothing until the next real ASCII delimiter.
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
    segments.push(DisplaySegment { raw, display: at..at, style: InlineStyle::Plain });
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
    segments.push(DisplaySegment { raw, display: display_start..display_end, style });
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
        assert_eq!(dl.segments, vec![DisplaySegment { raw: 0..line.len(), display: 0..line.len(), style: InlineStyle::Plain }]);
    }

    #[test]
    fn empty_line_is_a_single_empty_segment() {
        let dl = build_display_line("", false);
        assert_eq!(dl.display_text, "");
        assert_eq!(dl.segments, vec![DisplaySegment { raw: 0..0, display: 0..0, style: InlineStyle::Plain }]);
    }

    #[test]
    fn plain_line_with_no_constructs_is_unchanged() {
        let dl = build_display_line("just plain text", false);
        assert_eq!(dl.display_text, "just plain text");
        assert_contiguous(&dl, "just plain text".len());
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
}

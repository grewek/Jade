//! Building a [`DisplayLine`] from a raw line: headings, quotes, list markers,
//! cells, callout headers.

use super::*;

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
    build_display_line_with(line, active, &Macros::default())
}

/// [`build_display_line`], its formulas expanded with the note's `macros`.
pub fn build_display_line_with(line: &str, active: bool, macros: &Macros) -> DisplayLine {
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
            macros,
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
            macros,
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
            macros,
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
        macros,
        &mut segments,
        &mut display_text,
    );
    DisplayLine {
        display_text,
        segments,
        indent,
    }
}

/// A table cell's text with its markup shown as typed, as the cell being
/// edited is.
pub fn raw_cell_line(text: &str) -> DisplayLine {
    DisplayLine {
        display_text: text.to_string(),
        segments: vec![DisplaySegment {
            raw: 0..text.len(),
            display: 0..text.len(),
            style: InlineStyle::Plain,
            replaced: false,
        }],
        indent: LineIndent::None,
    }
}

/// A table cell's text with its inline markup applied (bold, code, links), as
/// one line. Unlike [`build_display_line`] it never reads a leading `>`, `-` or
/// `#` as a block: inside a cell they are just characters.
pub fn build_cell_line(text: &str) -> DisplayLine {
    let mut segments = Vec::new();
    let mut display_text = String::new();
    scan_inline_in(
        text,
        0,
        text.len(),
        InlineStyle::Plain,
        None,
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
pub(super) fn compound_inner(
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
#[cfg(test)]
pub fn build_display_line_in_context(line: &str, active: bool, ctx: LineContext) -> DisplayLine {
    build_display_line_in_context_with(line, active, ctx, &Macros::default())
}

/// [`build_display_line_in_context`], its formulas expanded with `macros`.
pub fn build_display_line_in_context_with(
    line: &str,
    active: bool,
    ctx: LineContext,
    macros: &Macros,
) -> DisplayLine {
    match ctx {
        LineContext::Plain => build_display_line_with(line, active, macros),
        LineContext::ListMarkerLine(depth) => {
            let mut display_line = build_display_line_with(line, active, macros);
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
            scan_inline(
                line,
                0,
                line.len(),
                style,
                macros,
                &mut segments,
                &mut display_text,
            );
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
pub(super) fn heading_prefix(line: &str) -> Option<(u8, usize)> {
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

/// Builds a callout header line's Live Preview rendering — the `> [!type]`
/// (plus optional fold marker/custom title) line itself, not its body
/// (which renders as ordinary blockquote content via
/// `build_display_line_in_context`, unchanged — see `preview/callout.rs`'s own
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
#[cfg(test)]
pub fn build_callout_header_line(line: &str, active: bool) -> DisplayLine {
    build_callout_header_line_with(line, active, &Macros::default())
}

/// [`build_callout_header_line`], its title's formulas expanded with `macros`.
pub fn build_callout_header_line_with(line: &str, active: bool, macros: &Macros) -> DisplayLine {
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
        return build_display_line_with(line, active, macros);
    };
    let content = &line[prefix_len..];
    let Some(header) = callout_header_prefix(content) else {
        return build_display_line_with(line, active, macros);
    };

    let mut segments = Vec::new();
    let mut display_text = String::new();

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
                WIDGET_BLANK,
                InlineStyle::Widget(Widget::CalloutIcon(header.kind)),
            );
            scan_inline(
                line,
                title_start,
                line.len(),
                InlineStyle::CalloutTitle,
                macros,
                &mut segments,
                &mut display_text,
            );
        }
        None => {
            let marker_end = prefix_len + header.marker_end;
            push_hidden(&mut segments, &display_text, 0..marker_end);
            let type_name = &content[header.type_name.clone()];
            push_replaced(
                &mut segments,
                &mut display_text,
                marker_end..marker_end,
                WIDGET_BLANK,
                InlineStyle::Widget(Widget::CalloutIcon(header.kind)),
            );
            push_replaced(
                &mut segments,
                &mut display_text,
                marker_end..line.len(),
                &capitalize(type_name),
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
pub(super) fn capitalize(text: &str) -> String {
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
pub(super) fn blockquote_prefix(line: &str) -> Option<(u8, usize)> {
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
pub(super) enum ListMarker {
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
pub(super) fn list_marker(line: &str) -> Option<(ListMarker, usize)> {
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
pub(super) fn indented_list_marker(line: &str) -> Option<(ListMarker, usize, usize)> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    let (marker, marker_len) = list_marker(&line[indent..])?;
    Some((marker, indent, marker_len))
}

/// Whether `line[offset..]` starts with a GFM task-list checkbox
/// (`[ ] `/`[x] `/`[X] `) — `Some(checked)` if so, else `None`. Pure
/// detector; [`push_list_marker`] does the actual rendering decision.
pub(super) fn task_checkbox_prefix(line: &str, offset: usize) -> Option<bool> {
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
/// `- [ ] `, not `"  " + "☐ "`) — matching Obsidian/GitHub's own task-list
/// rendering, where the checkbox *is* the marker, there's no separate
/// bullet dot alongside it. An **ordered** marker keeps its own number
/// (`"1. "`) and gets the checkbox glyph appended right after
/// (`"1. ☐ "`) — the number still carries meaning a checkbox alone
/// doesn't, so GFM task items on an ordered list keep it.
///
/// A click on the glyph toggles it: see [`DisplayLine::checkbox_at`].
pub(super) fn push_list_marker(
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
        push_replaced(
            segments,
            display_text,
            marker_start..marker_end + 4,
            WIDGET_BLANK,
            InlineStyle::Widget(Widget::Checkbox { checked }),
        );
        return marker_end + 4;
    }

    match marker {
        ListMarker::Unordered => push_replaced(
            segments,
            display_text,
            marker_start..marker_end,
            WIDGET_BLANK,
            InlineStyle::Widget(Widget::Bullet),
        ),
        ListMarker::Ordered { number } => push_replaced(
            segments,
            display_text,
            marker_start..marker_end,
            &format!("{number}. "),
            InlineStyle::ListMarker,
        ),
    }

    match checkbox {
        Some(checked) => {
            push_replaced(
                segments,
                display_text,
                marker_end..marker_end + 4,
                WIDGET_BLANK,
                InlineStyle::Widget(Widget::Checkbox { checked }),
            );
            marker_end + 4
        }
        None => marker_end,
    }
}

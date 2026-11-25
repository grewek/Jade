//! The box behind a fenced code block (`[code_block]`, #58): one rounded,
//! optionally outlined box from the block's first shown line to its last,
//! painted before the lines so their text and selection sit on it.

use super::*;
use crate::settings::Radius;
use crate::text_editor::preview::LineRole;

/// How code boxes look, once per frame.
#[derive(Clone, Copy)]
pub(super) struct CodeBoxLook {
    pub fill: Hsla,
    pub border: Option<(Pixels, Hsla)>,
    pub radius: Radius,
    /// em of the body font, above and below the block (paint-only).
    pub padding: f32,
    /// The colour of the language label, when `code_block.language_label` is on.
    pub label: Option<Hsla>,
}

/// The language of an opening fence line (```` ```rust ````): the first word of its
/// info string, `None` when there is none.
pub(in crate::text_editor) fn fence_language(line: &str) -> Option<&str> {
    let rest = line.trim_start().trim_start_matches(['`', '~']);
    // A backtick fence's info string has no backticks; the line is not a fence otherwise.
    rest.split_whitespace().next().filter(|w| !w.contains('`'))
}

/// The lines of the code block around line `i` (which has a fence role):
/// from its opening fence to the line after its closing one, or to the end
/// of the note when it is left open. `open_before(j)` is whether a fence is
/// open entering line `j`.
pub(in crate::text_editor) fn block_lines(
    roles: &[LineRole],
    open_before: impl Fn(usize) -> bool,
    i: usize,
) -> std::ops::Range<usize> {
    debug_assert!(!matches!(roles[i], LineRole::Plain));
    let mut start = i;
    while start > 0 && !(roles[start] == LineRole::FenceDelimiter && !open_before(start)) {
        start -= 1;
    }
    let mut end = i;
    while end < roles.len() {
        // A delimiter entered with a fence open closes it.
        let closes = roles[end] == LineRole::FenceDelimiter && open_before(end);
        end += 1;
        if closes {
            break;
        }
    }
    start..end
}

/// Paints the box of every code block in `range` (the visible lines).
pub(super) fn paint_code_boxes(
    range: &Range<usize>,
    (view, doc, text): (&FrameView, &DocumentCache, &str),
    look: &CodeBoxLook,
    (window, cx): (&mut Window, &mut App),
) {
    let roles = doc.line_roles();
    let (layout, bounds) = (&view.layout, view.bounds);
    let pad = view.font_size * look.padding;
    let mut i = range.start;
    while i < range.end.min(roles.len()) {
        if matches!(roles[i], LineRole::Plain) {
            i += 1;
            continue;
        }
        let block = block_lines(roles, |j| doc.fence_open_before(j), i);
        i = block.end.max(i + 1);
        // A block in a side-by-side region has no place of its own here.
        if doc.side_by_side(block.start).is_some() {
            continue;
        }
        let top = layout.screen_y(bounds.origin.y, block.start);
        let bottom = layout.screen_y(bounds.origin.y, block.end);
        // Nothing is shown of a block whose fences are hidden and which is empty.
        if bottom <= top {
            continue;
        }
        // Far off-screen the heights are estimates, and a quad that tall is
        // wasted: keep a margin past the editor.
        let margin = bounds.size.height;
        let (top, bottom) = (
            top.max(bounds.origin.y - margin),
            bottom.min(bounds.origin.y + bounds.size.height + margin),
        );
        // Horizontally the box is the editor's width; the lines are inset into it.
        let area = Bounds {
            origin: point(bounds.origin.x, top - pad),
            size: size(bounds.size.width, bottom - top + 2. * pad),
        };
        paint_box(area, look.radius, look.fill, look.border, window);
        if let Some(color) = look.label {
            paint_label(
                (view, doc, text),
                (block.start, top, area.right()),
                color,
                (window, cx),
            );
        }
    }
}

/// The block's language, small, in the top-right corner of its first row. Not
/// drawn while the opening fence is shown raw (the caret is on it) or has none.
fn paint_label(
    (view, doc, text): (&FrameView, &DocumentCache, &str),
    (line, top, right): (usize, Pixels, Pixels),
    color: Hsla,
    (window, cx): (&mut Window, &mut App),
) {
    let raw = &text[doc.line_start_byte(line)..doc.line_end_byte(line)];
    let on_it = view.focused
        && (doc.line_start_byte(line)..=doc.line_end_byte(line)).contains(&view.cursor);
    let Some(name) = fence_language(raw).filter(|_| !on_it) else {
        return;
    };
    let size = view.font_size * LABEL_SCALE;
    let run = TextRun {
        len: name.len(),
        font: view.font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(name.to_string().into(), size, &[run], None);
    let at = point(right - shaped.width - LABEL_INSET, top);
    let _ = shaped.paint(at, view.line_height, TextAlign::Left, None, window, cx);
}

/// The label is this much of the body size.
const LABEL_SCALE: f32 = 0.75;
/// Distance of the label from the box's right edge.
const LABEL_INSET: Pixels = px(10.);

#[cfg(test)]
mod tests {
    // Not `super::*`: its `gpui_kit::*` glob brings a `test` macro that shadows `#[test]`.
    use super::{LineRole, block_lines, fence_language};
    use crate::text_editor::preview::fence::classify_lines;

    fn blocks(text: &str) -> Vec<(usize, std::ops::Range<usize>)> {
        let roles = classify_lines(text);
        // The fence state entering each line, from the same classifier.
        let mut open = vec![false; roles.len()];
        let mut state = None;
        for (i, line) in text.lines().enumerate() {
            open[i] = state.is_some();
            state = crate::text_editor::preview::fence::step(state, line).1;
        }
        (0..roles.len())
            .filter(|&i| roles[i] != LineRole::Plain)
            .map(|i| (i, block_lines(&roles, |j| open[j], i)))
            .collect()
    }

    #[test]
    fn a_block_runs_from_its_opening_fence_to_after_its_closing_one() {
        let text = "a\n```rust\nx\ny\n```\nb\n";
        for (i, range) in blocks(text) {
            assert_eq!(range, 1..5, "line {i}");
        }
    }

    #[test]
    fn adjacent_blocks_are_told_apart() {
        let text = "```\na\n```\n```\nb\n```\n";
        let found = blocks(text);
        assert!(
            found
                .iter()
                .filter(|(i, _)| *i < 3)
                .all(|(_, r)| *r == (0..3))
        );
        assert!(
            found
                .iter()
                .filter(|(i, _)| *i >= 3)
                .all(|(_, r)| *r == (3..6))
        );
    }

    #[test]
    fn the_language_is_the_first_word_of_the_info_string() {
        assert_eq!(fence_language("```rust"), Some("rust"));
        assert_eq!(fence_language("  ~~~ python  title=x"), Some("python"));
        assert_eq!(fence_language("```"), None);
        assert_eq!(fence_language("````"), None);
    }

    #[test]
    fn an_unclosed_block_runs_to_the_end() {
        // The empty line after the final newline is a line of the open block too.
        let text = "a\n```\nx\ny\n";
        for (_, range) in blocks(text) {
            assert_eq!(range, 1..5);
        }
    }
}

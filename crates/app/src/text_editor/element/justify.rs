//! Justified text (Blocksatz): every wrapped row of a prose line but its last
//! is stretched so it ends at the wrap width, by widening its spaces evenly.
//!
//! GPUI's `WrappedLine::paint` only aligns left, centre or right, and its glyph
//! painter is private, so a justified line is painted row by row, word by word
//! (public `shape_line`, which GPUI caches), each word moved right by the
//! stretch of the spaces before it. Rows and row counts are untouched: wrapping
//! is GPUI's, only where a word is drawn changes. The caret, selection and
//! clicks use the same shift ([`position_for_index`], [`unshift_x`]).
//!
//! A row stays ragged when stretching it would open gaps wider than
//! [`MAX_STRETCH`] extra spaces (a short row before a long word), as a
//! typesetter would. Leading spaces (a list marker's blank columns) never move.

use super::*;

/// The most a space may grow, in widths of itself.
const MAX_STRETCH: f32 = 2.0;
/// Slack under this is not worth moving words for.
const MIN_SLACK: f32 = 0.5;

/// How one wrapped row is stretched. Byte offsets are into the display text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Plan {
    /// Where the row starts, and its first word and the end of its last.
    start: usize,
    first: usize,
    last: usize,
    /// The row's start in the unwrapped line, to turn glyph x into row x.
    x0: Pixels,
    /// Added to each space between the first and last word.
    extra: Pixels,
}

/// Whether a line is justified at all: prose that is shown rendered.
pub(super) fn eligible(state: &LineState, display_line: &DisplayLine, entries: usize) -> bool {
    state.justify
        && entries == 1
        && !state.active
        && !state.hidden
        && !state.column_frame
        && !state.callout_is_header
        && matches!(state.role, LineRole::Plain)
        && matches!(state.table_kind, TableLineKind::None)
        && !display_line
            .segments
            .iter()
            .any(|s| matches!(s.style, InlineStyle::Heading(_)))
}

/// The display byte where wrapped row `row` starts.
fn row_start(wrapped: &WrappedLine, row: usize) -> Option<usize> {
    let Some(i) = row.checked_sub(1) else {
        return Some(0);
    };
    let b = wrapped.wrap_boundaries().get(i)?;
    Some(wrapped.runs().get(b.run_ix)?.glyphs.get(b.glyph_ix)?.index)
}

/// How row `row` of `wrapped` is stretched; `None` for the last row of the line
/// and for rows that stay as they are.
pub(super) fn plan(wrapped: &WrappedLine, text: &str, row: usize) -> Option<Plan> {
    let width = wrapped.wrap_width?;
    if row >= wrapped.wrap_boundaries().len() {
        return None;
    }
    let (start, end) = (row_start(wrapped, row)?, row_start(wrapped, row + 1)?);
    let body = text.get(start..end)?;
    let first = start + (body.len() - body.trim_start_matches(' ').len());
    let last = start + body.trim_end_matches(' ').len();
    if first >= last {
        return None;
    }
    let gaps = text[first..last].bytes().filter(|&b| b == b' ').count();
    if gaps == 0 {
        return None;
    }
    let layout = &wrapped.unwrapped_layout;
    let x0 = layout.x_for_index(start);
    let slack = width - (layout.x_for_index(last) - x0);
    let space = text[first..last].find(' ').map(|at| {
        let at = first + at;
        layout.x_for_index(at + 1) - layout.x_for_index(at)
    })?;
    if slack <= px(MIN_SLACK) || slack / gaps as f32 > space * MAX_STRETCH {
        return None;
    }
    Some(Plan {
        start,
        first,
        last,
        x0,
        extra: slack / gaps as f32,
    })
}

impl Plan {
    /// How far right a character at display byte `index` of this row moves.
    fn shift(&self, text: &str, index: usize) -> Pixels {
        let upto = index.clamp(self.first, self.last);
        let spaces = text[self.first..upto]
            .bytes()
            .filter(|&b| b == b' ')
            .count();
        self.extra * spaces as f32
    }
}

/// `wrapped.position_for_index`, moved to where the justified row draws it.
pub(in crate::text_editor) fn position_for_index(
    (state, wrapped, display_line): (&LineState, &WrappedLine, &DisplayLine),
    entries: usize,
    index: usize,
    line_height: Pixels,
) -> Option<Point<Pixels>> {
    let mut pos = wrapped.position_for_index(index, line_height)?;
    if eligible(state, display_line, entries) {
        let row = (pos.y / line_height) as usize;
        if let Some(plan) = plan(wrapped, &display_line.display_text, row) {
            pos.x += plan.shift(&display_line.display_text, index);
        }
    }
    Some(pos)
}

/// The x a click at `x` (relative to the line, on `row`) would have had in the
/// unstretched row: the inverse of the shift, so the nearest character is found
/// by GPUI's own lookup. A click in a widened gap lands at the next word's start.
pub(super) fn unshift_x(
    (state, wrapped, display_line): (&LineState, &WrappedLine, &DisplayLine),
    entries: usize,
    (row, x): (usize, Pixels),
) -> Pixels {
    if !eligible(state, display_line, entries) {
        return x;
    }
    let text = &display_line.display_text;
    let Some(plan) = plan(wrapped, text, row) else {
        return x;
    };
    let layout = &wrapped.unwrapped_layout;
    let mut passed = 0;
    let spaces = (plan.first..plan.last).filter(|&at| text.as_bytes()[at] == b' ');
    for (n, at) in spaces.enumerate() {
        let next_word = layout.x_for_index(at + 1) - plan.x0;
        if x < next_word + plan.extra * (n + 1) as f32 {
            // In the gap before this space's following word: clamp into it.
            return (x - plan.extra * passed as f32).min(next_word);
        }
        passed = n + 1;
    }
    x - plan.extra * passed as f32
}

/// The runs of `runs` (which tile the display text in order) inside `range`.
fn slice_runs(runs: &[TextRun], range: &Range<usize>, out: &mut Vec<TextRun>) {
    out.clear();
    let mut at = 0;
    for run in runs {
        let (from, to) = (at.max(range.start), (at + run.len).min(range.end));
        if from < to {
            out.push(TextRun {
                len: to - from,
                ..run.clone()
            });
        }
        at += run.len;
    }
}

/// What a justified line needs to paint: where it sits and how it is styled.
pub(super) struct RowPaint<'a> {
    pub origin: Point<Pixels>,
    pub line_height: Pixels,
    pub font_size: Pixels,
    pub runs: &'a [TextRun],
}

/// Paints `wrapped` row by row, with the stretch of each justified row.
pub(super) fn paint(
    wrapped: &WrappedLine,
    text: &str,
    paint: &RowPaint,
    (window, cx): (&mut Window, &mut App),
) {
    let rows = wrapped.wrap_boundaries().len() + 1;
    let layout = &wrapped.unwrapped_layout;
    let mut scratch = Vec::with_capacity(paint.runs.len());
    let mut piece = |range: Range<usize>, x: Pixels, row: usize, window: &mut Window| {
        if range.is_empty() {
            return;
        }
        slice_runs(paint.runs, &range, &mut scratch);
        let shaped = window.text_system().shape_line(
            SharedString::from(text[range].to_owned()),
            paint.font_size,
            &scratch,
            None,
        );
        let at = paint.origin + point(x, paint.line_height * row as f32);
        let _ = shaped.paint(at, paint.line_height, TextAlign::Left, None, window, cx);
    };
    for row in 0..rows {
        let Some(start) = row_start(wrapped, row) else {
            continue;
        };
        let end = row_start(wrapped, row + 1).unwrap_or(text.len());
        let Some(plan) = plan(wrapped, text, row) else {
            piece(start..end, px(0.), row, window);
            continue;
        };
        let mut word_start = plan.first;
        let mut spaces = 0;
        for at in plan.first..=plan.last {
            if at == plan.last || text.as_bytes()[at] == b' ' {
                let x = layout.x_for_index(word_start) - plan.x0 + plan.extra * spaces as f32;
                piece(word_start..at, x, row, window);
                word_start = at + 1;
                spaces += 1;
            }
        }
    }
}

// Explicit imports, not `use super::*`: with `test-support` that glob also
// brings in GPUI's `test` macro, which shadows `#[test]` (see `element.rs`).
#[cfg(test)]
mod tests {
    use super::super::super::Editor;
    use super::super::{DisplayLine, LineState, shape_display_line};
    use super::{plan, position_for_index, unshift_x};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext, TestAppContext, WindowOptions, px};

    const PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog and then keeps \
        running through the quiet meadow until the evening settles over the hills";

    /// Runs `check` with the paragraph shaped at `width`, justified or not.
    fn with_shaped(
        cx: &mut TestAppContext,
        text: &'static str,
        (width, justify): (f32, bool),
        check: impl FnOnce(&LineState, &gpui_kit::WrappedLine, &DisplayLine, f32) + 'static,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::ThemeEngine::install(None, cx);
        });
        let (window, _editor) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Editor::new("", cx))
            })
            .expect("open test window")
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let style = window.text_style();
            let (font, size) = (style.font(), style.font_size.to_pixels(window.rem_size()));
            let state = LineState {
                justify,
                ..LineState::plain()
            };
            let mut results =
                shape_display_line(text, &state, &font, size, style.color, px(width), window);
            let (wrapped, display) = results.remove(0);
            check(&state, &wrapped, &display, window.line_height().into());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn every_row_but_the_last_ends_at_the_wrap_width(cx: &mut TestAppContext) {
        with_shaped(
            cx,
            PARAGRAPH,
            (300., true),
            |state, wrapped, display, lh| {
                let lh = px(lh);
                let rows = wrapped.wrap_boundaries().len() + 1;
                assert!(rows >= 3, "needs a few rows, got {rows}");
                let mut justified = 0;
                for row in 0..rows {
                    let p = plan(wrapped, &display.display_text, row);
                    let Some(p) = p else { continue };
                    justified += 1;
                    // The last word's end, as the caret draws it, reaches the edge.
                    let end = position_for_index((state, wrapped, display), 1, p.last, lh).unwrap();
                    assert!(
                        (end.x - px(300.)).abs() < px(0.6),
                        "row {row} ends at {:?}",
                        end.x
                    );
                }
                assert!(justified >= 1, "no row was justified");
                assert!(plan(wrapped, &display.display_text, rows - 1).is_none());
            },
        );
    }

    #[gpui_kit::test]
    fn nothing_moves_when_off_or_on_the_caret_line(cx: &mut TestAppContext) {
        with_shaped(
            cx,
            PARAGRAPH,
            (300., false),
            |state, wrapped, display, lh| {
                let lh = px(lh);
                let at = display.display_text.len() / 2;
                let plain = wrapped.position_for_index(at, lh).unwrap();
                let moved = position_for_index((state, wrapped, display), 1, at, lh).unwrap();
                assert_eq!(plain, moved);
                let active = LineState {
                    justify: true,
                    active: true,
                    ..LineState::plain()
                };
                let kept = position_for_index((&active, wrapped, display), 1, at, lh).unwrap();
                assert_eq!(plain, kept, "the caret's own raw line is not justified");
            },
        );
    }

    #[gpui_kit::test]
    fn a_click_maps_back_through_the_stretch(cx: &mut TestAppContext) {
        with_shaped(
            cx,
            PARAGRAPH,
            (300., true),
            |state, wrapped, display, lh| {
                let lh = px(lh);
                let text = &display.display_text;
                for row in 0..wrapped.wrap_boundaries().len() {
                    let Some(p) = plan(wrapped, text, row) else {
                        continue;
                    };
                    let words = (p.first..p.last).filter(|&i| text.as_bytes()[i] == b' ');
                    for space in words {
                        let at = space + 1;
                        let shown =
                            position_for_index((state, wrapped, display), 1, at, lh).unwrap();
                        let natural = wrapped.position_for_index(at, lh).unwrap();
                        let back = unshift_x((state, wrapped, display), 1, (row, shown.x));
                        assert!(
                            (back - natural.x).abs() < px(0.6),
                            "row {row} word at {at}: {back:?} vs {:?}",
                            natural.x
                        );
                    }
                }
            },
        );
    }
}

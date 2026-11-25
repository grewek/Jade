//! The chip behind inline code (`[inline_code]`, #58): a rounded box per row
//! the span's text sits on, painted under the text, selection and caret.

use super::*;
use crate::settings::Radius;
use crate::text_editor::preview::DisplayLine;

/// How chips look, once per frame: `None` of this when `inline_code.background`
/// is off.
#[derive(Clone, Copy)]
pub(super) struct ChipLook {
    pub fill: Hsla,
    /// em of the line's font, added left and right.
    pub padding: f32,
    pub radius: Radius,
    /// One character of the body font.
    pub char_width: Pixels,
}

/// A run of one span's characters on one row: layout-space y of the row, and
/// the x of its first character and of the end of its last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowRun {
    pub(super) y: Pixels,
    pub(super) from: Pixels,
    pub(super) to: Pixels,
}

/// Groups characters positioned at `(x, y)` (in order) into per-row runs, each
/// character `advance` wide.
fn row_runs(chars: impl Iterator<Item = (Pixels, Pixels)>, advance: Pixels) -> Vec<RowRun> {
    let mut runs: Vec<RowRun> = Vec::new();
    for (x, y) in chars {
        match runs.last_mut() {
            Some(run) if run.y == y => run.to = x + advance,
            _ => runs.push(RowRun {
                y,
                from: x,
                to: x + advance,
            }),
        }
    }
    runs
}

/// The row runs of every inline code span of the (rendered) line.
pub(super) fn code_runs(
    (state, wrapped, display_line): (&LineState, &WrappedLine, &DisplayLine),
    line_height: Pixels,
    advance: Pixels,
) -> Vec<RowRun> {
    // GPUI puts the first character of a wrapped row at the end of the row
    // before it; its place is the start of its own row.
    let rows = wrapped.wrap_boundaries().len();
    let at = |index: usize| {
        if let Some(row) = (1..=rows).find(|&r| justify::row_start(wrapped, r) == Some(index)) {
            return Some(point(px(0.), line_height * row as f32));
        }
        justify::position_for_index((state, wrapped, display_line), 1, index, line_height)
    };
    let mut out = Vec::new();
    for seg in display_line
        .segments
        .iter()
        .filter(|s| s.style == InlineStyle::Code && !s.display.is_empty())
    {
        let text = &display_line.display_text[seg.display.clone()];
        let last = seg.display.start + text.char_indices().next_back().map_or(0, |(i, _)| i);
        let (Some(first_pos), Some(last_pos)) = (at(seg.display.start), at(last)) else {
            continue;
        };
        // Usually the whole span is on one row: two lookups. Only a span the
        // wrap splits is walked character by character.
        if first_pos.y == last_pos.y {
            out.push(RowRun {
                y: first_pos.y,
                from: first_pos.x,
                to: last_pos.x + advance,
            });
        } else {
            let chars = text
                .char_indices()
                .filter_map(|(i, _)| at(seg.display.start + i))
                .map(|p| (p.x, p.y));
            out.extend(row_runs(chars, advance));
        }
    }
    out
}

/// Paints a chip for every inline code span of the (rendered) line.
pub(super) fn paint_code_chips(
    shaped: (&LineState, &WrappedLine, &DisplayLine),
    (entry, origin): (&EntryHeight, Point<Pixels>),
    look: &ChipLook,
    window: &mut Window,
) {
    let scale = shaped.2.font_scale();
    for run in code_runs(shaped, entry.row, look.char_width * scale) {
        paint_chip(run, (shaped.1, entry, origin), look, scale, window);
    }
}

fn paint_chip(
    run: RowRun,
    (wrapped, entry, origin): (&WrappedLine, &EntryHeight, Point<Pixels>),
    look: &ChipLook,
    scale: f32,
    window: &mut Window,
) {
    let (placed, row_height) = entry.place(point(run.from, run.y));
    // The text box GPUI centres in the row.
    let text_height = wrapped.ascent() + wrapped.descent();
    let pad = look.padding * f32::from(look.char_width) / crate::text_editor::math::MONO_ADVANCE_EM
        * scale;
    let bounds = Bounds {
        origin: origin
            + point(
                placed.x - px(pad),
                placed.y + (row_height - text_height) / 2.,
            ),
        size: size(run.to - run.from + px(2. * pad), text_height),
    };
    paint_box(bounds, look.radius, look.fill, None, window);
}

#[cfg(test)]
mod tests {
    // Not `super::*`: its `gpui_kit::*` glob brings a `test` macro that shadows `#[test]`.
    use super::{RowRun, row_runs};
    use gpui_kit::px;

    #[test]
    fn characters_on_one_row_make_one_run() {
        let chars = [(px(10.), px(0.)), (px(20.), px(0.)), (px(30.), px(0.))];
        let runs = row_runs(chars.into_iter(), px(10.));
        assert_eq!(
            runs,
            [RowRun {
                y: px(0.),
                from: px(10.),
                to: px(40.)
            }]
        );
    }

    #[test]
    fn a_wrapped_span_makes_a_run_per_row() {
        let chars = [
            (px(90.), px(0.)),
            (px(100.), px(0.)),
            (px(0.), px(26.)),
            (px(10.), px(26.)),
        ];
        let runs = row_runs(chars.into_iter(), px(10.));
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].from, runs[0].to), (px(90.), px(110.)));
        assert_eq!((runs[1].from, runs[1].to), (px(0.), px(20.)));
    }
}

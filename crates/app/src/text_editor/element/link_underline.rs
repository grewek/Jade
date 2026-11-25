//! The underline under links (`[link]`, #58): one line per row the link's text
//! is on, painted under the text; `Hover` draws only the link the pointer is on.

use super::code_chip::{RowRun, style_runs};
use super::*;
use crate::settings::Underline;
use crate::text_editor::preview::DisplayLine;

/// How links are underlined, once per frame; `None` of this when both kinds are `Never`.
#[derive(Clone, Copy)]
pub(super) struct UnderlineLook {
    pub color: Hsla,
    pub links: Underline,
    pub wikilinks: Underline,
    /// One character of the body font.
    pub char_width: Pixels,
}

/// The link under the pointer: its line and byte span within the line.
pub(crate) type HoverLink = (usize, std::ops::Range<usize>);

/// Whether a segment of `style` is underlined: always, never, or only when it
/// lies in the hovered link.
pub(super) fn wanted(
    look: &UnderlineLook,
    (style, raw): (InlineStyle, &std::ops::Range<usize>),
    (line, hover): (usize, Option<&HoverLink>),
) -> bool {
    let mode = match style {
        InlineStyle::Link => look.links,
        InlineStyle::WikiLink => look.wikilinks,
        _ => return false,
    };
    match mode {
        Underline::Always => true,
        Underline::Never => false,
        Underline::Hover => hover
            .is_some_and(|(l, span)| *l == line && span.start <= raw.start && raw.end <= span.end),
    }
}

/// Paints the underlines of the (rendered) line.
pub(super) fn paint_link_underlines(
    shaped: (&LineState, &WrappedLine, &DisplayLine),
    (entry, origin): (&EntryHeight, Point<Pixels>),
    (look, line, hover): (&UnderlineLook, usize, Option<&HoverLink>),
    window: &mut Window,
) {
    let scale = shaped.2.font_scale();
    let runs = style_runs(shaped, entry.row, look.char_width * scale, |s| {
        wanted(look, (s.style, &s.raw), (line, hover))
    });
    for run in runs {
        paint_underline(run, (shaped.1, entry, origin), look.color, window);
    }
}

fn paint_underline(
    run: RowRun,
    (wrapped, entry, origin): (&WrappedLine, &EntryHeight, Point<Pixels>),
    color: Hsla,
    window: &mut Window,
) {
    let (placed, row_height) = entry.place(point(run.from, run.y));
    let text_height = wrapped.ascent() + wrapped.descent();
    // Just under the baseline.
    let y = placed.y + (row_height - text_height) / 2. + wrapped.ascent() + UNDERLINE_GAP;
    window.paint_quad(fill(
        Bounds {
            origin: origin + point(placed.x, y),
            size: size(run.to - run.from, UNDERLINE_THICKNESS),
        },
        color,
    ));
}

/// Space between the baseline and the line.
const UNDERLINE_GAP: Pixels = px(2.);
const UNDERLINE_THICKNESS: Pixels = px(1.);

#[cfg(test)]
mod tests {
    // Not `super::*`: its `gpui_kit::*` glob brings a `test` macro that shadows `#[test]`.
    use super::{UnderlineLook, wanted};
    use crate::settings::Underline;
    use crate::text_editor::preview::InlineStyle;
    use gpui_kit::{black, px};

    fn look(links: Underline, wikilinks: Underline) -> UnderlineLook {
        UnderlineLook {
            color: black(),
            links,
            wikilinks,
            char_width: px(10.),
        }
    }

    fn seg(
        style: InlineStyle,
        raw: std::ops::Range<usize>,
    ) -> (InlineStyle, std::ops::Range<usize>) {
        (style, raw)
    }

    fn as_ref(s: &(InlineStyle, std::ops::Range<usize>)) -> (InlineStyle, &std::ops::Range<usize>) {
        (s.0, &s.1)
    }

    #[test]
    fn each_kind_follows_its_own_mode() {
        let l = look(Underline::Always, Underline::Never);
        assert!(wanted(&l, as_ref(&seg(InlineStyle::Link, 0..4)), (0, None)));
        assert!(!wanted(
            &l,
            as_ref(&seg(InlineStyle::WikiLink, 0..4)),
            (0, None)
        ));
        assert!(!wanted(
            &l,
            as_ref(&seg(InlineStyle::Plain, 0..4)),
            (0, None)
        ));
    }

    #[test]
    fn hover_underlines_only_the_hovered_link() {
        let l = look(Underline::Hover, Underline::Hover);
        let hover = (3, 10..30);
        let inside = seg(InlineStyle::Link, 11..20);
        let other = seg(InlineStyle::Link, 40..50);
        assert!(wanted(&l, as_ref(&inside), (3, Some(&hover))));
        assert!(!wanted(&l, as_ref(&other), (3, Some(&hover))));
        assert!(
            !wanted(&l, as_ref(&inside), (4, Some(&hover))),
            "another line"
        );
        assert!(!wanted(&l, as_ref(&inside), (3, None)));
    }
}

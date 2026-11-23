//! Painting the drawn controls (bullets, checkboxes, callout icons) and image
//! blocks over the blank columns and rows the layout reserved for them.

#[allow(unused_imports)]
use super::*;
use crate::text_editor::math;
use crate::text_editor::preview::DisplaySegment;

/// Colors of the controls drawn over a line's widget columns.
pub(super) struct WidgetColors {
    /// An unchecked box's outline.
    pub(super) border: Hsla,
    /// A checked box's fill, and its tick over it.
    pub(super) accent: Hsla,
    pub(super) on_accent: Hsla,
}

/// What painting a line's widgets needs besides the line.
pub(super) struct WidgetPaint<'a> {
    pub(super) colors: &'a WidgetColors,
    pub(super) inline: &'a InlineColors,
    pub(super) font_size: Pixels,
    pub(super) line_height: Pixels,
    /// The raw text `display_line`'s segments index into.
    pub(super) line_text: &'a str,
    /// How wide the line's rows are (a display formula is centred in it).
    pub(super) row_width: Pixels,
}

/// Paints a bundled Lucide icon (`path`) filling `bounds`.
pub(super) fn paint_icon(
    path: &'static str,
    bounds: Bounds<Pixels>,
    color: Hsla,
    (window, cx): (&mut Window, &mut App),
) {
    let _ = window.paint_svg(
        bounds,
        path.into(),
        None,
        TransformationMatrix::unit(),
        color,
        cx,
    );
}

/// Draws each [`Widget`] segment of `display_line` over the blank columns it
/// reserved: `origin` is the line's top left, `wrapped` its shaped text, which
/// says where those columns are.
pub(super) fn paint_widgets(
    display_line: &DisplayLine,
    wrapped: &WrappedLine,
    origin: Point<Pixels>,
    paint: &WidgetPaint,
    (window, cx): (&mut Window, &mut App),
) {
    let WidgetPaint {
        colors,
        inline,
        font_size,
        line_height,
        ..
    } = *paint;
    for seg in &display_line.segments {
        if seg.style == InlineStyle::Math {
            paint_math(display_line, seg, wrapped, origin, paint, (window, cx));
            continue;
        }
        let InlineStyle::Widget(widget) = seg.style else {
            continue;
        };
        let (Some(from), Some(to)) = (
            wrapped.position_for_index(seg.display.start, line_height),
            wrapped.position_for_index(seg.display.end, line_height),
        ) else {
            continue;
        };
        let cell = Bounds {
            origin: origin + from,
            size: size(to.x - from.x, line_height),
        };
        // Centered vertically on the row, left aligned in the cell.
        let square = |side: Pixels| Bounds {
            origin: cell.origin + point(px(1.), (line_height - side) / 2.),
            size: size(side, side),
        };
        match widget {
            Widget::Bullet => {
                let side = (font_size * 0.3).max(px(4.));
                let dot = Bounds {
                    origin: cell.origin
                        + point(
                            (cell.size.width / 2. - side) / 2. + px(1.),
                            (line_height - side) / 2.,
                        ),
                    size: size(side, side),
                };
                window.paint_quad(quad(
                    dot,
                    side / 2.,
                    inline.subtle,
                    px(0.),
                    Hsla::transparent_black(),
                    BorderStyle::Solid,
                ));
            }
            Widget::Checkbox { checked } => {
                let boxed = square((font_size * 0.9).round());
                let (fill_color, edge) = if checked {
                    (colors.accent, colors.accent)
                } else {
                    (Hsla::transparent_black(), colors.border)
                };
                window.paint_quad(quad(
                    boxed,
                    CHECKBOX_RADIUS,
                    fill_color,
                    CHECKBOX_BORDER,
                    edge,
                    BorderStyle::Solid,
                ));
                if checked {
                    let inset = boxed.size.width * 0.12;
                    let tick = Bounds {
                        origin: boxed.origin + point(inset, inset),
                        size: size(
                            boxed.size.width - inset * 2.,
                            boxed.size.height - inset * 2.,
                        ),
                    };
                    paint_icon("icons/check.svg", tick, colors.on_accent, (window, cx));
                }
            }
            Widget::CalloutIcon(kind) => {
                let icon = square(font_size);
                paint_icon(
                    crate::text_editor::preview::callout::callout_icon(kind),
                    icon,
                    inline.callout(kind),
                    (window, cx),
                );
            }
        }
    }
}

/// The family name the bundled math font registers under.
const MATH_FONT_FAMILY: &str = "Latin Modern Math";

/// Draws a formula's glyphs and rules over the blank columns `seg` reserved,
/// left aligned on the text's baseline; a `$$…$$` that is the whole line is
/// centred in the row instead. A formula the wrap split across rows is left
/// blank: there is no one place to draw it.
fn paint_math(
    display_line: &DisplayLine,
    seg: &DisplaySegment,
    wrapped: &WrappedLine,
    origin: Point<Pixels>,
    paint: &WidgetPaint,
    (window, cx): (&mut Window, &mut App),
) {
    let _ = (display_line, cx);
    let Some(raw) = paint.line_text.get(seg.raw.clone()) else {
        return;
    };
    let Some(rendered) = math::rendered_span(raw) else {
        return;
    };
    let line_height = paint.line_height;
    let (Some(from), Some(to)) = (
        wrapped.position_for_index(seg.display.start, line_height),
        wrapped.position_for_index(seg.display.end, line_height),
    ) else {
        return;
    };
    if from.y != to.y {
        return;
    }
    let font_id = window.text_system().resolve_font(&font(MATH_FONT_FAMILY));
    // GPUI centres a row's text in the line height; the baseline follows.
    let pad = (line_height - (wrapped.ascent() + wrapped.descent())) / 2.;
    let em = paint.font_size * math::MATH_SCALE;
    let centred = raw.starts_with("$$") && paint.line_text.trim() == raw;
    let left = if centred {
        ((paint.row_width - em * rendered.layout.width) / 2.).max(px(0.))
    } else {
        from.x
    };
    let baseline = origin + point(left, from.y + pad + wrapped.ascent());
    let (ink, error) = (paint.inline.text, paint.inline.broken);
    for item in &rendered.layout.items {
        match *item {
            math::Item::Glyph {
                id,
                x,
                y,
                size,
                error: failed,
            } => {
                let _ = window.paint_glyph(
                    baseline + point(em * x, em * y),
                    font_id,
                    GlyphId(u32::from(id)),
                    em * size,
                    if failed { error } else { ink },
                );
            }
            math::Item::Rule {
                x,
                y,
                width,
                height,
            } => {
                // At least a device pixel, or a thin bar can vanish.
                let thick = (em * height).max(px(1.));
                window.paint_quad(fill(
                    Bounds {
                        origin: baseline + point(em * x, em * y),
                        size: size(em * width, thick),
                    },
                    ink,
                ));
            }
        }
    }
}

/// Corner radius and outline width of a task checkbox.
pub(super) const CHECKBOX_RADIUS: Pixels = px(3.);

pub(super) const CHECKBOX_BORDER: Pixels = px(1.5);

/// Draws the image of `info` at the top left of `area`, starting its decode if
/// there is none yet. `false` while there is nothing to draw (or it cannot be
/// decoded), so the placeholder stands in.
pub(super) fn paint_image(
    info: &ImageInfo,
    requested: Option<u32>,
    area: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let (width, height) = image::display_size(info.size, requested, area.size.width);
    let device_width = (width * window.scale_factor()).ceil() as u32;
    let lookup =
        cx.default_global::<ImageCache>()
            .lookup(&info.path, device_width, info.size.width);
    if let Some(decode_at) = lookup.decode_at {
        let path = info.path.clone();
        window
            .spawn(cx, async move |cx| {
                let job = cx.background_executor().spawn({
                    let path = path.clone();
                    async move { image::decode(&path, decode_at) }
                });
                let decoded = job.await;
                cx.update(|window, cx| {
                    cx.default_global::<ImageCache>().finish(&path, decoded);
                    window.refresh();
                })
                .ok();
            })
            .detach();
    }
    let Some(image) = lookup.image else {
        return false;
    };
    let frame = Bounds {
        origin: area.origin + point(px(0.), px(2.)),
        size: size(px(width), px(height)),
    };
    window
        .paint_image(frame, frame, Corners::default(), image, 0, false)
        .is_ok()
}

/// A bordered box with the file name, standing where an image block's rows are
/// until the image itself is drawn.
pub(super) fn paint_image_placeholder(
    path: &str,
    area: Bounds<Pixels>,
    (font, font_size): (&Font, Pixels),
    (border, label): (Hsla, Hsla),
    (window, cx): (&mut Window, &mut App),
) {
    let frame = Bounds {
        origin: area.origin + point(px(0.), px(2.)),
        size: size(area.size.width, area.size.height - px(4.)),
    };
    window.paint_quad(outline(frame, border, BorderStyle::Solid));
    let name = path.rsplit('/').next().unwrap_or(path);
    let run = TextRun {
        len: name.len(),
        font: font.clone(),
        color: label,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(name.to_string().into(), font_size, &[run], None);
    let line_height = window.line_height();
    let _ = shaped.paint(
        frame.origin + point(px(8.), px(4.)),
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

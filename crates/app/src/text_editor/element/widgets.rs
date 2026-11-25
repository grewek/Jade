//! Painting the drawn controls (bullets, checkboxes, callout icons) and image
//! blocks over the blank columns and rows the layout reserved for them.

#[allow(unused_imports)]
use super::*;
use crate::text_editor::math::{self, Macros};
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
    /// The heights of the line's rows.
    pub(super) entry: &'a EntryHeight,
    /// The raw text `display_line`'s segments index into.
    pub(super) line_text: &'a str,
    /// How wide the line's rows are (a display formula is centred in it).
    pub(super) row_width: Pixels,
    /// The note's macros, which its formulas are expanded with.
    pub(super) macros: &'a Macros,
    /// The corners of a task checkbox (`task.checkbox_radius`).
    pub(super) checkbox_radius: crate::settings::Radius,
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
        entry,
        ..
    } = *paint;
    let line_height = entry.row;
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
        let (from, line_height) = entry.place(from);
        let to = entry.place(to).0;
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
                paint_box(
                    boxed,
                    paint.checkbox_radius,
                    fill_color,
                    Some((CHECKBOX_BORDER, edge)),
                    window,
                );
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
    let _ = cx;
    let Some(raw) = paint.line_text.get(seg.raw.clone()) else {
        return;
    };
    let Some(rendered) = math::rendered_span(raw, paint.macros) else {
        return;
    };
    let entry = paint.entry;
    let (Some(from), Some(to)) = (
        wrapped.position_for_index(seg.display.start, entry.row),
        wrapped.position_for_index(seg.display.end, entry.row),
    ) else {
        return;
    };
    if from.y != to.y {
        return;
    }
    let (from, line_height) = entry.place(from);
    let font_id = window.text_system().resolve_font(&font(MATH_FONT_FAMILY));
    // GPUI centres a row's text in the line height; the baseline follows.
    let pad = (line_height - (wrapped.ascent() + wrapped.descent())) / 2.;
    // A formula in a heading follows the heading's size.
    let em = paint.font_size * display_line.font_scale() * math::scale();
    let centred = raw.starts_with("$$") && paint.line_text.trim() == raw;
    let left = if centred {
        ((paint.row_width - em * rendered.layout.width) / 2.).max(px(0.))
    } else {
        from.x
    };
    let baseline = origin + point(left, from.y + pad + wrapped.ascent());
    let colors = (paint.inline.text, paint.inline.broken);
    paint_layout(&rendered.layout, baseline, (em, font_id), colors, window);
}

/// Draws the glyphs and rules of `layout` with its baseline origin at `baseline`;
/// `em` is the size of 1em in px. `colors` = (ink, error ink).
fn paint_layout(
    layout: &math::Layout,
    baseline: Point<Pixels>,
    (em, font_id): (Pixels, FontId),
    (ink, error): (Hsla, Hsla),
    window: &mut Window,
) {
    for item in &layout.items {
        match *item {
            math::Item::Glyph {
                id,
                x,
                y,
                size,
                error: failed,
                color,
            } => {
                let _ = window.paint_glyph(
                    baseline + point(em * x, em * y),
                    font_id,
                    GlyphId(u32::from(id)),
                    em * size,
                    if failed { error } else { tint(color, ink) },
                );
            }
            math::Item::Line {
                x,
                y,
                x2,
                y2,
                thickness,
                color,
            } => {
                let mut stroke = PathBuilder::stroke((em * thickness).max(px(1.)));
                stroke.move_to(baseline + point(em * x, em * y));
                stroke.line_to(baseline + point(em * x2, em * y2));
                if let Ok(path) = stroke.build() {
                    window.paint_path(path, tint(color, ink));
                }
            }
            math::Item::Rule {
                x,
                y,
                width,
                height,
                color,
            } => {
                let thick = bar_thickness(em * height, window.scale_factor());
                window.paint_quad(fill(
                    Bounds {
                        origin: baseline + point(em * x, em * y),
                        size: size(em * width, thick),
                    },
                    tint(color, ink),
                ));
            }
        }
    }
}

/// A bar's drawn thickness: its own, but at least one device pixel, or a thin bar
/// can vanish. Not a logical pixel, which is two device pixels on a scaled
/// display and makes the bars of small nested fractions touch their digits (#50).
fn bar_thickness(thickness: Pixels, scale_factor: f32) -> Pixels {
    thickness.max(px(1.) / scale_factor.max(1.))
}

/// `color` (`0xRRGGBB`) from a `\color`, or the text colour.
fn tint(color: Option<u32>, ink: Hsla) -> Hsla {
    color.map_or(ink, |rgb| {
        let [r, g, b] = math::channels(rgb);
        gpui_kit::Rgba { r, g, b, a: 1.0 }.into()
    })
}

/// Paints the display formula `body` of a `$$` block centred in `area`.
pub(super) fn paint_math_block(
    body: &str,
    area: Bounds<Pixels>,
    font_size: Pixels,
    colors: (Hsla, Hsla),
    macros: &Macros,
    (window, _cx): (&mut Window, &mut App),
) {
    let Some(rendered) = math::rendered(body, true, macros) else {
        return;
    };
    let font_id = window.text_system().resolve_font(&font(MATH_FONT_FAMILY));
    let em = font_size * math::scale();
    let layout = &rendered.layout;
    let mut left = ((area.size.width - em * layout.width) / 2.).max(px(0.));
    let top = (area.size.height - em * (layout.ascent + layout.descent)) / 2.;
    if let Some(tag) = &layout.tag {
        // The number sits at the right edge, centred vertically like the
        // formula; a formula wide enough to reach it moves left.
        let gap = em * 0.5;
        let tag_left = (area.size.width - em * tag.width).max(px(0.));
        let tag_top = (area.size.height - em * (tag.ascent + tag.descent)) / 2.;
        let tag_base = area.origin + point(tag_left, tag_top + em * tag.ascent);
        paint_layout(tag, tag_base, (em, font_id), colors, window);
        left = left.min((tag_left - gap - em * layout.width).max(px(0.)));
    }
    let baseline = area.origin + point(left, top + em * layout.ascent);
    paint_layout(layout, baseline, (em, font_id), colors, window);
}

/// Outline width of a task checkbox.
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
    let line_height = crate::text_editor::settings::body_row_height();
    let _ = shaped.paint(
        frame.origin + point(px(8.), px(4.)),
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

#[cfg(test)]
mod bar_tests {
    // Not `use super::*`: with `test-support` that glob brings in GPUI's `test`
    // macro, which shadows `#[test]` (see the note in `element.rs`).
    use super::bar_thickness;
    use gpui_kit::px;

    #[test]
    fn a_thin_bar_is_a_device_pixel_not_a_logical_one() {
        assert_eq!(bar_thickness(px(0.2), 1.0), px(1.));
        assert_eq!(bar_thickness(px(0.2), 2.0), px(0.5));
        assert_eq!(bar_thickness(px(0.2), 0.0), px(1.), "an unknown scale is 1");
    }

    #[test]
    fn a_thick_bar_keeps_its_thickness() {
        assert_eq!(bar_thickness(px(1.5), 2.0), px(1.5));
    }
}

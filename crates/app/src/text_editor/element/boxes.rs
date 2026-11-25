//! One way to draw a box with rounded corners and an outline, so the radius
//! and border width come from `[shape]` and the element's own `radius` key
//! (#58) instead of a constant at each site.

use crate::settings::Radius;
use gpui_kit::*;

/// What a box's outline and corners are: from [`Typography`](crate::settings::Typography)
/// (`radius_of(..)` and `shape.border_width`).
#[derive(Clone, Copy)]
pub(super) struct BoxShape {
    pub radius: Radius,
    pub border_width: f32,
}

/// `radius` for a box of `bounds`: clamped to half the shorter side.
fn corner(radius: Radius, bounds: Bounds<Pixels>) -> Pixels {
    let shorter = f32::from(bounds.size.width.min(bounds.size.height));
    px(radius.px(shorter))
}

/// Paints `bounds` filled with `fill`, outlined with `border` (width px,
/// colour) when there is one, all four corners rounded by `radius`.
pub(super) fn paint_box(
    bounds: Bounds<Pixels>,
    radius: Radius,
    fill: Hsla,
    border: Option<(Pixels, Hsla)>,
    window: &mut Window,
) {
    let r = corner(radius, bounds);
    paint_box_corners(bounds, Corners::all(r), fill, border, window);
}

/// [`paint_box`] with each corner's own radius (a table's header band is
/// rounded on top only).
pub(super) fn paint_box_corners(
    bounds: Bounds<Pixels>,
    corners: Corners<Pixels>,
    fill: Hsla,
    border: Option<(Pixels, Hsla)>,
    window: &mut Window,
) {
    let (width, color) = border.unwrap_or((px(0.), Hsla::transparent_black()));
    window.paint_quad(quad(
        bounds,
        corners,
        fill,
        width,
        color,
        BorderStyle::Solid,
    ));
}

/// The outline, in px from the box's top left, of the part of a `size` box with
/// corners of `radius` that lies between x = `x0` and `x1` (`x0 < x1 <= width`):
/// the strip's top and bottom edges follow the rounded corners.
fn strip_outline(size: (f32, f32), radius: f32, (x0, x1): (f32, f32)) -> Vec<(f32, f32)> {
    const STEPS: usize = 6;
    // How far the box's edge is below its top, at `x` from its left.
    let drop = |x: f32| {
        if radius <= 0.0 || x >= radius {
            return 0.0;
        }
        let across = radius - x;
        radius - (radius * radius - across * across).max(0.0).sqrt()
    };
    let xs: Vec<f32> = (0..=STEPS)
        .map(|k| x0 + (x1 - x0) * k as f32 / STEPS as f32)
        .collect();
    let mut points: Vec<(f32, f32)> = xs.iter().map(|&x| (x, drop(x))).collect();
    points.extend(xs.iter().rev().map(|&x| (x, size.1 - drop(x))));
    points
}

/// Paints the vertical strip `x0..x1` (px from the left of `bounds`) of a box
/// with rounded corners `radius`, in `color`: an accent bar that follows the
/// box's corner instead of sticking out of it.
pub(super) fn paint_strip(
    bounds: Bounds<Pixels>,
    radius: Radius,
    (x0, x1): (Pixels, Pixels),
    color: Hsla,
    window: &mut Window,
) {
    let r = corner(radius, bounds);
    if r <= px(0.) {
        let strip = Bounds {
            origin: bounds.origin + point(x0, px(0.)),
            size: size(x1 - x0, bounds.size.height),
        };
        window.paint_quad(fill(strip, color));
        return;
    }
    let outline = strip_outline(
        (f32::from(bounds.size.width), f32::from(bounds.size.height)),
        f32::from(r),
        (f32::from(x0), f32::from(x1)),
    );
    let mut path = PathBuilder::fill();
    for (i, &(x, y)) in outline.iter().enumerate() {
        let at = bounds.origin + point(px(x), px(y));
        if i == 0 {
            path.move_to(at);
        } else {
            path.line_to(at);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: its `gpui_kit::*` glob brings a `test` macro that shadows `#[test]`.
    use super::{Radius, corner, strip_outline};
    use gpui_kit::{Bounds, Pixels, point, px, size};

    fn square(side: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(side), px(side)),
        }
    }

    #[test]
    fn a_strip_follows_the_corner_it_sits_in() {
        // Square corners: a plain rectangle.
        let flat = strip_outline((100., 50.), 0., (4., 7.));
        assert!(flat.iter().all(|&(_, y)| y == 0. || y == 50.));
        // Radius 8, strip 4..7: the top edge drops where the corner curves
        // away (1.07px at x = 4), less further in, and the bottom mirrors it.
        let round = strip_outline((100., 50.), 8., (4., 7.));
        assert!((round[0].1 - 1.0718).abs() < 1e-3, "{:?}", round[0]);
        assert!(round[6].1 < round[0].1 && round[6].1 > 0.);
        let bottom = round.last().copied().unwrap();
        assert!((bottom.1 - (50. - round[0].1)).abs() < 1e-3);
        // Past the corner the edge is straight again.
        let past = strip_outline((100., 50.), 8., (9., 12.));
        assert!(past.iter().all(|&(_, y)| y == 0. || y == 50.));
        // A strip inside the corner never leaves the box.
        assert!(round.iter().all(|&(_, y)| (0.0..=50.0).contains(&y)));
    }

    #[test]
    fn the_corner_never_passes_half_the_shorter_side() {
        let wide = Bounds {
            size: size(px(100.), px(10.)),
            ..square(0.)
        };
        assert_eq!(corner(Radius::Px(6.), wide), px(5.));
        assert_eq!(corner(Radius::Px(6.), square(40.)), px(6.));
        assert_eq!(corner(Radius::Full, wide), px(5.));
        assert_eq!(corner(Radius::Px(0.), wide), px(0.));
    }
}

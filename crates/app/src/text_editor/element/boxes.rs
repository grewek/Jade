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

#[cfg(test)]
mod tests {
    // Not `super::*`: its `gpui_kit::*` glob brings a `test` macro that shadows `#[test]`.
    use super::{Radius, corner};
    use gpui_kit::{Bounds, Pixels, point, px, size};

    fn square(side: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(side), px(side)),
        }
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

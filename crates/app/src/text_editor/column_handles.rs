//! The controls on a side-by-side region: a grip on each divider to drag the
//! ratio, a `+` to add a column and a button to unwrap, both above the region's
//! right end. Pure geometry in pixels from the region's top left (like
//! `table_handles.rs`), so what is drawn and what a press hits share numbers.

/// Radius of the round buttons.
pub(super) const RADIUS: f32 = 8.0;
/// A divider grip's long and short side.
pub(super) const GRIP_LONG: f32 = 22.0;
pub(super) const GRIP_SHORT: f32 = 10.0;
/// How far above the region the pointer still counts as over it, so the
/// buttons straddling its top edge can be reached.
pub(super) const MARGIN: f32 = 2.0 * RADIUS + 4.0;

/// A region's size and where its dividers are, from its top left.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Frame {
    /// The x of each divider.
    pub dividers: Vec<f32>,
    pub width: f32,
    pub height: f32,
    /// Another column can be added.
    pub can_add: bool,
    /// The `+` and unwrap buttons show: not for a float block, which has only
    /// its grips.
    pub buttons: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    /// The grip on the divider between column `k` and `k + 1`.
    Divider(usize),
    Add,
    Unwrap,
}

/// A divider being dragged: which region, which divider, and the shares the
/// pointer would give now (applied on release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ColumnDrag {
    pub region: usize,
    pub divider: usize,
    pub percent: Vec<u8>,
}

/// Whether the controls show for the pointer at `p`.
pub(super) fn hover_at(frame: &Frame, p: (f32, f32)) -> bool {
    (0.0..=frame.width).contains(&p.0) && (-MARGIN..=frame.height).contains(&p.1)
}

/// The controls to draw: every divider's grip, `+` unless at the limit, unwrap.
pub(super) fn shown(frame: &Frame) -> Vec<Handle> {
    let mut out: Vec<Handle> = (0..frame.dividers.len()).map(Handle::Divider).collect();
    if !frame.buttons {
        return out;
    }
    if frame.can_add {
        out.push(Handle::Add);
    }
    out.push(Handle::Unwrap);
    out
}

/// A control's rectangle as `(x, y, width, height)`.
pub(super) fn rect_of(frame: &Frame, handle: Handle) -> (f32, f32, f32, f32) {
    let centered = |cx: f32, cy: f32, w: f32, h: f32| (cx - w / 2.0, cy - h / 2.0, w, h);
    let round = |cx: f32| centered(cx, -RADIUS, 2.0 * RADIUS, 2.0 * RADIUS);
    match handle {
        Handle::Divider(k) => {
            // Near the top, but inside a short region.
            let y = (frame.height / 2.0).min(GRIP_LONG);
            centered(frame.dividers[k], y, GRIP_SHORT, GRIP_LONG)
        }
        Handle::Unwrap => round(frame.width - RADIUS),
        Handle::Add => round(frame.width - 3.0 * RADIUS - 4.0),
    }
}

/// Which control the point is on.
pub(super) fn handle_at(frame: &Frame, p: (f32, f32)) -> Option<Handle> {
    shown(frame).into_iter().find(|&handle| {
        let (x, y, w, h) = rect_of(frame, handle);
        (x..=x + w).contains(&p.0) && (y..=y + h).contains(&p.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(can_add: bool) -> Frame {
        Frame {
            dividers: vec![300.0],
            width: 600.0,
            height: 100.0,
            can_add,
            buttons: true,
        }
    }

    #[test]
    fn controls_show_over_the_region_and_just_above_it() {
        let f = frame(true);
        assert!(hover_at(&f, (10.0, 50.0)));
        assert!(hover_at(&f, (10.0, -MARGIN + 1.0)));
        assert!(!hover_at(&f, (10.0, -MARGIN - 1.0)));
        assert!(!hover_at(&f, (10.0, 101.0)));
        assert!(!hover_at(&f, (-1.0, 50.0)));
    }

    #[test]
    fn each_control_is_found_at_its_own_centre() {
        let f = frame(true);
        for handle in shown(&f) {
            let (x, y, w, h) = rect_of(&f, handle);
            assert_eq!(handle_at(&f, (x + w / 2.0, y + h / 2.0)), Some(handle));
        }
        assert_eq!(handle_at(&f, (100.0, 50.0)), None);
    }

    #[test]
    fn no_add_control_at_the_column_limit() {
        let f = frame(false);
        assert!(!shown(&f).contains(&Handle::Add));
        assert_eq!(shown(&f), [Handle::Divider(0), Handle::Unwrap]);
    }

    #[test]
    fn a_short_region_keeps_its_grip_inside() {
        let mut f = frame(true);
        f.height = 20.0;
        let (_, y, _, h) = rect_of(&f, Handle::Divider(0));
        assert!(y + h / 2.0 <= f.height);
    }
}

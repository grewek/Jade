//! The controls on a side-by-side region: a grip on each divider to drag the
//! ratio, a `+` to add a column and a button to unwrap, both above the region's
//! right end. Pure geometry in pixels from the region's top left (like
//! `table/handles.rs`), so what is drawn and what a press hits share numbers.

/// Radius of the round buttons.
pub(in crate::text_editor) const RADIUS: f32 = 8.0;
/// A divider grip's long and short side.
pub(in crate::text_editor) const GRIP_LONG: f32 = 22.0;
pub(in crate::text_editor) const GRIP_SHORT: f32 = 10.0;
/// How far above the region the pointer still counts as over it, so the
/// buttons straddling its top edge can be reached.
pub(in crate::text_editor) const MARGIN: f32 = 2.0 * RADIUS + 4.0;

/// A region's size and where its dividers are, from its top left.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::text_editor) struct Frame {
    /// The x of each divider.
    pub dividers: Vec<f32>,
    pub width: f32,
    pub height: f32,
    /// Another column can be added.
    pub can_add: bool,
    /// The `+` and unwrap buttons show: not for a float block, which has only
    /// its grips.
    pub buttons: bool,
    /// Where each float of an aside has its grip: `(divider, centre y)`. Empty
    /// (the grip near the region's top) for columns.
    pub grips: Vec<(usize, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::text_editor) enum Handle {
    /// The grip on the divider between column `k` and `k + 1`.
    Divider(usize),
    Add,
    Unwrap,
}

/// A divider being dragged: which region, which divider, and the shares the
/// pointer would give now (applied on release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::text_editor) struct ColumnDrag {
    pub region: usize,
    pub divider: usize,
    pub percent: Vec<u8>,
}

/// Whether the controls show for the pointer at `p`.
pub(in crate::text_editor) fn hover_at(frame: &Frame, p: (f32, f32)) -> bool {
    (0.0..=frame.width).contains(&p.0) && (-MARGIN..=frame.height).contains(&p.1)
}

/// The controls to draw: every divider's grip, `+` unless at the limit, unwrap.
pub(in crate::text_editor) fn shown(frame: &Frame) -> Vec<Handle> {
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
pub(in crate::text_editor) fn rect_of(frame: &Frame, handle: Handle) -> (f32, f32, f32, f32) {
    let centered = |cx: f32, cy: f32, w: f32, h: f32| (cx - w / 2.0, cy - h / 2.0, w, h);
    let round = |cx: f32| centered(cx, -RADIUS, 2.0 * RADIUS, 2.0 * RADIUS);
    match handle {
        Handle::Divider(k) => {
            // Near the top, but inside a short region; the first float's when
            // there are grips beside floats.
            let y = frame
                .grips
                .iter()
                .find(|&&(d, _)| d == k)
                .map_or((frame.height / 2.0).min(GRIP_LONG), |&(_, y)| y);
            centered(frame.dividers[k], y, GRIP_SHORT, GRIP_LONG)
        }
        Handle::Unwrap => round(frame.width - RADIUS),
        Handle::Add => round(frame.width - 3.0 * RADIUS - 4.0),
    }
}

/// Every place a control is drawn: a divider has a grip beside each float.
pub(in crate::text_editor) fn rects_of(frame: &Frame, handle: Handle) -> Vec<(f32, f32, f32, f32)> {
    let Handle::Divider(k) = handle else {
        return vec![rect_of(frame, handle)];
    };
    let at: Vec<_> = frame
        .grips
        .iter()
        .filter(|&&(d, _)| d == k)
        .map(|&(_, y)| {
            (
                frame.dividers[k] - GRIP_SHORT / 2.,
                y - GRIP_LONG / 2.,
                GRIP_SHORT,
                GRIP_LONG,
            )
        })
        .collect();
    if at.is_empty() {
        vec![rect_of(frame, handle)]
    } else {
        at
    }
}

/// Which control the point is on.
pub(in crate::text_editor) fn handle_at(frame: &Frame, p: (f32, f32)) -> Option<Handle> {
    shown(frame).into_iter().find(|&handle| {
        rects_of(frame, handle)
            .into_iter()
            .any(|(x, y, w, h)| (x..=x + w).contains(&p.0) && (y..=y + h).contains(&p.1))
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
            grips: Vec::new(),
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

    /// A float block's region is as tall as its text: each float has its own
    /// grip on the divider, and none shows between them.
    #[test]
    fn a_divider_has_a_grip_beside_each_float() {
        let mut f = frame(false);
        f.height = 2000.0;
        f.grips = vec![(0, 30.0), (0, 900.0)];
        let spots: Vec<_> = rects_of(&f, Handle::Divider(0));
        assert_eq!(spots.len(), 2);
        for (x, y, w, h) in spots {
            assert_eq!(
                handle_at(&f, (x + w / 2.0, y + h / 2.0)),
                Some(Handle::Divider(0))
            );
        }
        assert_eq!(handle_at(&f, (300.0, 500.0)), None, "between the floats");
        // The first float's grip is the one a drag's ghost is drawn at.
        let (_, y, _, h) = rect_of(&f, Handle::Divider(0));
        assert_eq!(y + h / 2.0, 30.0);
    }
}

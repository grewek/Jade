//! The element: viewport-sized, so layout never needs the document's height.

use std::rc::Rc;
use std::time::Instant;

use gpui_kit::{
    App, Bounds, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseDownEvent,
    Pixels, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, Window,
    fill, hsla, point, px, relative, size,
};

use super::anchor::Anchor;
use super::{SpikeView, State, trace};

/// Bound on the blocks drawn in one frame (the smallest block is 22 px).
const MAX_VISIBLE: usize = 1_000;
const TICK: f32 = 20.;
const FLING_START: f64 = 120.;
/// Share of the speed a fling keeps from one frame to the next.
const FLING_DECAY: f64 = 0.97;
const FLING_END: f64 = 0.3;

pub(crate) struct Drawn {
    pub(crate) block: usize,
    /// Window coordinates.
    pub(crate) bounds: Bounds<Pixels>,
    label: ShapedLine,
}

pub(crate) struct Snapshot {
    pub(crate) anchor: Anchor,
    pub(crate) blocks: Vec<Drawn>,
    hitbox: Hitbox,
}

pub(crate) struct ScrollElement {
    view: Entity<SpikeView>,
    prepaint_us: u128,
}

impl ScrollElement {
    pub(crate) fn new(view: Entity<SpikeView>) -> Self {
        Self {
            view,
            prepaint_us: 0,
        }
    }
}

impl IntoElement for ScrollElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static LAST: std::cell::RefCell<Option<Rc<Snapshot>>> =
        const { std::cell::RefCell::new(None) };
}

impl Element for ScrollElement {
    type RequestLayoutState = ();
    type PrepaintState = Rc<Snapshot>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Rc<Snapshot> {
        let start = Instant::now();
        let view = self.view.read(cx);
        let (handle, focus) = (view.handle.clone(), view.focus.clone());
        window.set_focus_handle(&focus, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let mut state = handle.0.borrow_mut();
        state.viewport = bounds;
        state.frame += 1;
        if state.fling.abs() > FLING_END {
            state.pending.delta += state.fling;
            state.fling *= FLING_DECAY;
        } else {
            state.fling = 0.;
        }
        let remeasure_start = Instant::now();
        if state.remeasure {
            state.remeasure_above();
        }
        let remeasure_us = remeasure_start.elapsed().as_micros();
        let anchor = state.settle(f64::from(f32::from(bounds.size.height)));
        let blocks = lay_out(&state, anchor, bounds, window);
        if state.remeasure || state.fling != 0. {
            window.request_animation_frame();
        }
        self.prepaint_us = start.elapsed().as_micros() - remeasure_us;
        let first = blocks.get(1).or(blocks.first());
        trace::log(&format!(
            "frame={} interval_us={} prepaint_us={} remeasure_us={remeasure_us} anchor={}+{:.3} \
             above_px={:.1} total_px={:.1} tracked={} tracked_y={:.4} drawn={}",
            state.frame,
            trace::interval_us(start),
            self.prepaint_us,
            anchor.block,
            anchor.offset,
            state.heights.before(anchor.block),
            state.heights.total(),
            first.map_or(0, |d| d.block),
            first.map_or(0., |d| f32::from(d.bounds.origin.y)),
            blocks.len(),
        ));
        Rc::new(Snapshot {
            anchor,
            blocks,
            hitbox,
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        snapshot: &mut Rc<Snapshot>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let start = Instant::now();
        register_listeners(&self.view, snapshot, window, cx);
        for drawn in &snapshot.blocks {
            paint_block(drawn, window, cx);
        }
        trace::log(&format!("paint_us={}", start.elapsed().as_micros()));
        #[cfg(test)]
        LAST.with(|last| *last.borrow_mut() = Some(snapshot.clone()));
    }
}

fn lay_out(
    state: &State,
    anchor: Anchor,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) -> Vec<Drawn> {
    let style = window.text_style();
    // Relative to the anchor only: no total and no sum from the top.
    let mut y = -anchor.offset;
    let mut blocks = Vec::new();
    let last = state.heights.len().min(anchor.block + MAX_VISIBLE);
    for block in anchor.block..last {
        if y >= f64::from(f32::from(bounds.size.height)) {
            break;
        }
        let height = state.heights.height(block);
        let text: SharedString = format!("block {block}  ({height:.1} px)").into();
        let run = style.to_run(text.len());
        let label = window.text_system().shape_line(text, px(13.), &[run], None);
        blocks.push(Drawn {
            block,
            bounds: Bounds::new(
                point(bounds.origin.x, bounds.origin.y + px(y as f32)),
                size(bounds.size.width, px(height as f32)),
            ),
            label,
        });
        y += height;
    }
    blocks
}

/// A tinted box with a ruler inside, so that one pixel of drift shows.
fn paint_block(drawn: &Drawn, window: &mut Window, cx: &mut App) {
    let hue = (drawn.block % 12) as f32 / 12.;
    let area = drawn.bounds;
    window.paint_quad(fill(area, hsla(hue, 0.5, 0.82, 1.)));
    let line = size(area.size.width, px(2.));
    window.paint_quad(fill(
        Bounds::new(area.origin, line),
        hsla(hue, 0.7, 0.3, 1.),
    ));
    let view = window.content_mask().bounds;
    let mut y = px(TICK);
    while y < area.size.height {
        let at = point(area.origin.x, area.origin.y + y);
        if at.y > view.bottom() {
            break;
        }
        if at.y >= view.top() {
            let tick = size(px(24.), px(1.));
            window.paint_quad(fill(Bounds::new(at, tick), hsla(0., 0., 0.2, 1.)));
        }
        y += px(TICK);
    }
    let at = area.origin + point(px(32.), px(3.));
    if let Err(err) = drawn
        .label
        .paint(at, px(16.), TextAlign::Left, None, window, cx)
    {
        eprintln!("[spike] paint failed: {err}");
    }
}

/// Input only records what was asked for; prepaint applies it.
fn register_listeners(
    view: &Entity<SpikeView>,
    snapshot: &Rc<Snapshot>,
    window: &mut Window,
    cx: &mut App,
) {
    let (handle, focus) = {
        let view = view.read(cx);
        (view.handle.clone(), view.focus.clone())
    };
    let line_height = window.line_height();
    let (target, frame, id) = (handle.clone(), snapshot.clone(), view.entity_id());
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !frame.hitbox.should_handle_scroll(window) {
            return;
        }
        let delta = event.delta.pixel_delta(line_height).y;
        let kind = match event.delta {
            ScrollDelta::Pixels(raw) => format!("pixels {:.3}", f32::from(raw.y)),
            ScrollDelta::Lines(raw) => format!("lines {:.3}", raw.y),
        };
        trace::log(&format!("wheel {kind} phase={:?}", event.touch_phase));
        let mut state = target.0.borrow_mut();
        state.fling = 0.;
        state.pending.delta -= f64::from(f32::from(delta));
        cx.notify(id);
    });

    let (frame, target_focus) = (snapshot.clone(), focus.clone());
    window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && frame.hitbox.is_hovered(window) {
            window.focus(&target_focus, cx);
        }
    });

    let id = view.entity_id();
    window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let mut state = handle.0.borrow_mut();
        match event.keystroke.key.as_str() {
            "pagedown" => state.pending.pages += 1,
            "pageup" => state.pending.pages -= 1,
            "down" => state.pending.delta += 40.,
            "up" => state.pending.delta -= 40.,
            "home" => state.pending.jump = Some(0.),
            "end" => state.pending.jump = Some(state.heights.total()),
            "m" => state.pending.jump = Some(state.heights.total() / 2.),
            "g" => state.pending.reveal = Some(state.heights.len() / 4),
            "r" => state.remeasure = !state.remeasure,
            "f" => state.fling = FLING_START,
            "b" => state.fling = -FLING_START,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify(id);
    });
}

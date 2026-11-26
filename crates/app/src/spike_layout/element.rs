//! The element: layout asks for the width, prepaint does all layout work,
//! paint reads the snapshot and registers this frame's listeners.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::*;

use super::trace::SharedTrace;
use super::{
    FONT_SIZE, KEY_CONTEXT, LINE_HEIGHT, Mode, ShapedLine_, Snapshot, SpikeNarrower, SpikeTab,
    SpikeView, SpikeWider,
};

/// Bound on the paragraphs shaped for one frame.
const MAX_SHAPED_LINES: usize = 2_000;
const RESIZE_STEP: Pixels = px(120.);

pub(crate) struct SpikeElement {
    view: Entity<SpikeView>,
    pub(crate) trace: SharedTrace,
}

impl SpikeElement {
    pub(crate) fn new(view: Entity<SpikeView>) -> Self {
        Self {
            view,
            trace: SharedTrace::default(),
        }
    }
}

impl IntoElement for SpikeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for SpikeElement {
    type RequestLayoutState = ();
    /// Immutable: paint cannot change layout through it.
    type PrepaintState = Rc<Snapshot>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mode = self.view.read(cx).mode;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if mode == Mode::Fill {
            style.size.height = relative(1.).into();
            return (window.request_layout(style, [], cx), ());
        }
        style.min_size.height = relative(1.).into();
        let view = self.view.clone();
        let trace = self.trace.clone();
        // Per-frame memo: taffy may ask several times with the same width.
        let memo = RefCell::new(Vec::<(Pixels, Pixels)>::new());
        let layout_id = window.request_measured_layout(style, move |known, available, _, cx| {
            let start = Instant::now();
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => px(0.),
            });
            let mut trace = trace.borrow_mut();
            let height = if width > px(0.) {
                let cached = memo.borrow().iter().find(|(w, _)| *w == width).copied();
                let height = cached.map_or_else(
                    || {
                        // Reads the view; changes nothing.
                        let height = view.read(cx).total_height(width);
                        memo.borrow_mut().push((width, height));
                        height
                    },
                    |(_, height)| height,
                );
                trace.measured_widths.push(width);
                trace.measured_height = Some(height);
                height
            } else {
                px(0.)
            };
            trace
                .measures
                .push((known.width, available.width, start.elapsed().as_micros()));
            size(width, height)
        });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Rc<Snapshot> {
        let start = Instant::now();
        let focus = self.view.read(cx).focus.clone();
        window.set_focus_handle(&focus, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // The clip is the visible part in either mode.
        let visible = window.content_mask().bounds.intersect(&bounds);
        let trace = self.trace.clone();
        let snapshot = self.view.update(cx, |view, _| {
            let mut trace = trace.borrow_mut();
            trace.previous_width = view.width;
            let width = bounds.size.width;
            if width != view.width {
                view.exact.fill(0.);
                view.width = width;
            }
            view.frame += 1;
            let (lines, caret) = lay_out(view, bounds, visible, window);
            let snapshot = Rc::new(Snapshot {
                bounds,
                lines,
                caret,
                hitbox,
            });
            view.snapshot = Some(snapshot.clone());
            trace.frame = view.frame;
            trace.prepaint_width = width;
            trace.shaped_width = view.width;
            trace.shaped_lines = snapshot.lines.len();
            trace.height_after_prepaint = view.total_height(width);
            snapshot
        });
        self.trace.borrow_mut().prepaint_time = start.elapsed();
        snapshot
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        snapshot: &mut Rc<Snapshot>,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(debug_assertions)]
        let before = self.view.read(cx).state_hash();
        let start = Instant::now();
        assert_eq!(bounds, snapshot.bounds, "paint got other bounds");
        let focus = self.view.read(cx).focus.clone();

        let mut key_context = KeyContext::default();
        key_context.add(KEY_CONTEXT);
        window.set_key_context(key_context);
        register_actions(&self.view, window);
        register_keys(&self.view, window);
        register_mouse(&self.view, snapshot, window);
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &snapshot.hitbox);

        for line in &snapshot.lines {
            let painted =
                line.wrapped
                    .paint(line.origin, LINE_HEIGHT, TextAlign::Left, None, window, cx);
            if let Err(err) = painted {
                eprintln!("[spike] paint failed: {err}");
            }
        }
        if let Some(caret) = snapshot.caret
            && focus.is_focused(window)
        {
            window.paint_quad(fill(caret, window.text_style().color));
        }
        if self.view.read(cx).paint_mutates {
            self.view.update(cx, |view, _| view.frame += 1);
        }
        {
            let mut trace = self.trace.borrow_mut();
            trace.paint_width = bounds.size.width;
            trace.paint_time = start.elapsed();
            trace.log();
            #[cfg(test)]
            super::trace::keep_last(&trace);
        }
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            before,
            self.view.read(cx).state_hash(),
            "paint changed layout state"
        );
    }
}

/// Shapes the visible paragraphs plus one screen ahead and records their exact
/// heights.
fn lay_out(
    view: &mut SpikeView,
    bounds: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    window: &mut Window,
) -> (Vec<ShapedLine_>, Option<Bounds<Pixels>>) {
    let width = bounds.size.width;
    let top = match view.mode {
        Mode::Measured => f32::from(visible.origin.y - bounds.origin.y).max(0.),
        Mode::Fill => f32::from(view.scroll_y),
    };
    let end = top + 2. * f32::from(visible.size.height);
    let content_top = match view.mode {
        Mode::Measured => bounds.origin.y,
        Mode::Fill => bounds.origin.y - view.scroll_y,
    };
    let style = window.text_style();
    let (first, mut y) = view.line_at(top, width);
    let mut shaped = Vec::new();
    let mut caret = None;
    let last = view.lines.len().min(first + MAX_SHAPED_LINES);
    for index in first..last {
        if y > end {
            break;
        }
        let text: SharedString = view.lines[index].clone().into();
        let run = style.to_run(text.len());
        let result = window
            .text_system()
            .shape_text(text, FONT_SIZE, &[run], Some(width), None);
        let Some(wrapped) = result.ok().and_then(|lines| lines.into_iter().next()) else {
            eprintln!("[spike] cannot shape line {index}");
            continue;
        };
        let origin = point(bounds.origin.x, content_top + px(y));
        let height = f32::from(wrapped.size(LINE_HEIGHT).height);
        view.exact[index] = height;
        if index == view.cursor.0
            && let Some(at) = wrapped.position_for_index(view.cursor.1, LINE_HEIGHT)
        {
            caret = Some(Bounds::new(origin + at, size(px(2.), LINE_HEIGHT)));
        }
        shaped.push(ShapedLine_ {
            index,
            origin,
            wrapped,
        });
        y += height;
    }
    (shaped, caret)
}

fn register_actions(view: &Entity<SpikeView>, window: &mut Window) {
    let target = view.clone();
    window.on_action(
        std::any::TypeId::of::<SpikeTab>(),
        move |_, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                cx.stop_propagation();
                target.update(cx, |view, cx| {
                    view.tabs_handled += 1;
                    let at = view.cursor.1;
                    view.replace(at..at, "    ");
                    window.invalidate_character_coordinates();
                    cx.notify();
                });
            }
        },
    );
    for (action, step) in [
        (std::any::TypeId::of::<SpikeNarrower>(), -RESIZE_STEP),
        (std::any::TypeId::of::<SpikeWider>(), RESIZE_STEP),
    ] {
        window.on_action(action, move |_, phase, window, _| {
            if phase == DispatchPhase::Bubble {
                let mut next = window.viewport_size();
                next.width += step;
                window.resize(next);
            }
        });
    }
}

fn register_keys(view: &Entity<SpikeView>, window: &mut Window) {
    let target = view.clone();
    window.on_key_event(move |event: &KeyDownEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let handled = target.update(cx, |view, cx| {
            match event.keystroke.key.as_str() {
                "backspace" => view.backspace(),
                "enter" => {
                    let at = view.cursor.1;
                    view.replace(at..at, "\n");
                }
                "left" => view.step(false),
                "right" => view.step(true),
                _ => return false,
            }
            cx.notify();
            true
        });
        if handled {
            cx.stop_propagation();
        }
    });
}

/// Clicks and the wheel read the snapshot of the frame that is on screen.
fn register_mouse(view: &Entity<SpikeView>, snapshot: &Rc<Snapshot>, window: &mut Window) {
    let (target, frame) = (view.clone(), snapshot.clone());
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !frame.hitbox.is_hovered(window) {
            return;
        }
        let hit = frame.lines.iter().find(|line| {
            let height = line.wrapped.size(LINE_HEIGHT).height;
            event.position.y >= line.origin.y && event.position.y < line.origin.y + height
        });
        let focus = target.read(cx).focus.clone();
        window.focus(&focus, cx);
        if let Some(line) = hit {
            let local = event.position - line.origin;
            let byte = line
                .wrapped
                .closest_index_for_position(local, LINE_HEIGHT)
                .unwrap_or_else(|nearest| nearest);
            target.update(cx, |view, cx| {
                view.cursor = (line.index, byte);
                view.marked = None;
                cx.notify();
            });
        }
        cx.stop_propagation();
    });

    let (target, frame) = (view.clone(), snapshot.clone());
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !frame.hitbox.should_handle_scroll(window) {
            return;
        }
        target.update(cx, |view, cx| {
            if view.mode == Mode::Fill {
                let delta = event.delta.pixel_delta(LINE_HEIGHT).y;
                view.scroll_y = (view.scroll_y - delta).max(px(0.));
                cx.notify();
            }
        });
    });
}

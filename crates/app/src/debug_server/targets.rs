//! Named elements and their last-painted bounds, so a client can ask what is
//! on screen and click "the Big row" instead of guessing coordinates.
//!
//! `.debug_name(..)` wraps an element in [`Named`], which forwards everything
//! to it (no layout node) and records its bounds during prepaint. The registry
//! is rebuilt every frame (`begin_frame` from the root view's render), and is
//! only touched when the debug server is on.

use std::sync::Mutex;

use gpui_kit::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, Window,
};
use jade_debug_protocol::{Point, Rect, Target};

/// More than this many named elements in one frame are dropped.
const MAX_TARGETS: usize = 512;

struct Entry {
    name: SharedString,
    bounds: Bounds<Pixels>,
    /// `bounds` clipped by the ancestors' content mask; empty when hidden.
    visible: Bounds<Pixels>,
}

static REGISTRY: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// Forgets the previous frame's elements (keeping the allocation).
pub(crate) fn begin_frame() {
    if !super::enabled() {
        return;
    }
    if let Ok(mut registry) = REGISTRY.lock() {
        registry.clear();
    }
}

fn record(name: &SharedString, bounds: Bounds<Pixels>, window: &Window) {
    let visible = bounds.intersect(&window.content_mask().bounds);
    let Ok(mut registry) = REGISTRY.lock() else {
        return;
    };
    if registry.len() < MAX_TARGETS {
        registry.push(Entry {
            name: name.clone(),
            bounds,
            visible,
        });
    }
}

fn is_empty(bounds: &Bounds<Pixels>) -> bool {
    bounds.size.width <= Pixels::ZERO || bounds.size.height <= Pixels::ZERO
}

/// Every element painted in the last frame, in paint order.
pub(super) fn list() -> Vec<Target> {
    let Ok(registry) = REGISTRY.lock() else {
        return Vec::new();
    };
    registry
        .iter()
        .map(|e| Target {
            name: e.name.to_string(),
            x: e.bounds.origin.x.into(),
            y: e.bounds.origin.y.into(),
            width: e.bounds.size.width.into(),
            height: e.bounds.size.height.into(),
            visible: !is_empty(&e.visible),
        })
        .collect()
}

/// The visible part of the one element called `name`, in window coordinates.
pub(super) fn visible_bounds(name: &str) -> Result<Rect, String> {
    let Ok(registry) = REGISTRY.lock() else {
        return Err("target registry poisoned".into());
    };
    let entry = unique(&registry, name)?;
    let r = entry.visible;
    Ok(Rect {
        x: r.origin.x.into(),
        y: r.origin.y.into(),
        w: r.size.width.into(),
        h: r.size.height.into(),
    })
}

/// The one entry called `name`, or why there is not exactly one visible.
fn unique<'a>(registry: &'a [Entry], name: &str) -> Result<&'a Entry, String> {
    let mut matches = registry.iter().filter(|e| &*e.name == name);
    let (Some(entry), None) = (matches.next(), matches.next()) else {
        let count = registry.iter().filter(|e| &*e.name == name).count();
        return Err(match count {
            0 => format!("no element named {name:?} was painted last frame (see list_targets)"),
            n => format!("{n} elements are named {name:?}; a click needs exactly one"),
        });
    };
    if is_empty(&entry.visible) {
        return Err(format!(
            "element {name:?} is scrolled or clipped out of view"
        ));
    }
    Ok(entry)
}

/// Center of the visible part of the one element called `name`.
pub(super) fn resolve(name: &str) -> Result<Point, String> {
    let Ok(registry) = REGISTRY.lock() else {
        return Err("target registry poisoned".into());
    };
    resolve_in(&registry, name)
}

fn resolve_in(registry: &[Entry], name: &str) -> Result<Point, String> {
    let entry = unique(registry, name)?;
    let center = entry.visible.center();
    Ok(Point {
        x: center.x.into(),
        y: center.y.into(),
    })
}

/// `.debug_name(..)` for anything that becomes an element.
pub(crate) trait DebugName: IntoElement + Sized {
    /// Names the element; use `debug_name_with` when the name must be built.
    fn debug_name(self, name: &'static str) -> Named<Self::Element> {
        self.debug_name_with(|| SharedString::from(name))
    }

    /// The name is built only when the debug server is on.
    fn debug_name_with(self, name: impl FnOnce() -> SharedString) -> Named<Self::Element> {
        Named {
            inner: self.into_element(),
            name: super::enabled().then(name),
        }
    }
}

impl<E: IntoElement> DebugName for E {}

pub(crate) struct Named<E> {
    inner: E,
    name: Option<SharedString>,
}

impl<E: Element> IntoElement for Named<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: Element> Element for Named<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.inner.id()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(name) = &self.name {
            record(name, bounds, window);
        }
        self.inner
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px, size};

    fn entry(name: &'static str, x: f32, visible_h: f32) -> Entry {
        let bounds = Bounds::new(point(px(x), px(0.)), size(px(100.), px(40.)));
        let visible = Bounds::new(point(px(x), px(0.)), size(px(100.), px(visible_h)));
        Entry {
            name: name.into(),
            bounds,
            visible,
        }
    }

    #[test]
    fn a_unique_visible_element_resolves_to_the_center_of_its_visible_part() {
        let registry = [entry("a", 10., 40.), entry("b", 200., 10.)];
        let a = resolve_in(&registry, "a").unwrap();
        assert_eq!((a.x, a.y), (60.0, 20.0));
        let b = resolve_in(&registry, "b").unwrap();
        assert_eq!((b.x, b.y), (250.0, 5.0), "aims at the part inside the clip");
    }

    #[test]
    fn unknown_ambiguous_and_clipped_names_are_errors_that_say_why() {
        let registry = [
            entry("dup", 0., 40.),
            entry("dup", 50., 40.),
            entry("gone", 0., 0.),
        ];
        assert!(
            resolve_in(&registry, "nope")
                .unwrap_err()
                .contains("list_targets")
        );
        assert!(
            resolve_in(&registry, "dup")
                .unwrap_err()
                .contains("2 elements")
        );
        assert!(
            resolve_in(&registry, "gone")
                .unwrap_err()
                .contains("out of view")
        );
    }
}

//! SPIKE #135 (throwaway, never merged): a viewport-sized element that owns
//! scrolling over 100,000 synthetic blocks. Enabled with
//! `JADE_SPIKE_SCROLL=anchor` (or `pixels`, the control that stores an
//! absolute pixel offset instead).

mod anchor;
mod element;
mod heights;
mod trace;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement, Pixels,
    Point, Render, Size, Styled, Window, div, point, px, size,
};

use anchor::Anchor;
pub(crate) use element::ScrollElement;
use heights::HeightMap;

pub(crate) const BLOCKS: usize = 100_000;
/// Blocks right above the anchor and random blocks anywhere above it that a
/// remeasuring frame gives new heights.
const REMEASURE_NEAR: usize = 200;
const REMEASURE_FAR: usize = 1_000;

/// Deterministic random numbers (xorshift64*).
pub(crate) struct Random(u64);

impl Random {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % bound.max(1)
    }

    /// Mostly paragraph-sized, some large, a few taller than any viewport.
    /// Tenths of a pixel, so that positions are fractional.
    pub(crate) fn height(&mut self) -> f64 {
        let (low, high) = match self.below(100) {
            0 => (10_000, 30_000),
            1..=9 => (2_000, 6_000),
            _ => (220, 2_000),
        };
        (low + self.below(high - low)) as f64 / 10.
    }
}

/// What input asked for since the last frame. Prepaint applies it, so layout
/// state changes nowhere else.
#[derive(Default)]
pub(crate) struct Pending {
    /// Wheel and touchpad, px, positive is down the document.
    pub(crate) delta: f64,
    /// Pages, positive is down.
    pub(crate) pages: i32,
    /// A content y from the scrollbar, or an end of the document.
    pub(crate) jump: Option<f64>,
    pub(crate) reveal: Option<usize>,
}

pub(crate) struct State {
    pub(crate) heights: HeightMap,
    pub(crate) anchor: Anchor,
    /// Control mode: the position is this content y, as a scroll offset is.
    pub(crate) pixel_position: Option<f64>,
    pub(crate) pending: Pending,
    pub(crate) viewport: Bounds<Pixels>,
    pub(crate) remeasure: bool,
    /// A running synthetic fling: px per frame.
    pub(crate) fling: f64,
    pub(crate) random: Random,
    pub(crate) frame: u64,
}

impl State {
    pub(crate) fn new(blocks: usize, control: bool) -> Self {
        let mut random = Random::new(135);
        let heights = (0..blocks).map(|_| random.height()).collect();
        Self {
            heights: HeightMap::new(heights),
            anchor: Anchor::default(),
            pixel_position: control.then_some(0.),
            pending: Pending::default(),
            viewport: Bounds::default(),
            remeasure: false,
            fling: 0.,
            random,
            frame: 0,
        }
    }

    /// New random heights for blocks strictly above the anchor.
    pub(crate) fn remeasure_above(&mut self) {
        let above = self.anchor.block;
        for block in above.saturating_sub(REMEASURE_NEAR)..above {
            let height = self.random.height();
            self.heights.set(block, height);
        }
        for _ in 0..REMEASURE_FAR.min(above) {
            let block = self.random.below(above as u64) as usize;
            let height = self.random.height();
            self.heights.set(block, height);
        }
    }

    /// Applies what is pending and returns the anchor to draw from.
    pub(crate) fn settle(&mut self, viewport: f64) -> Anchor {
        let pending = std::mem::take(&mut self.pending);
        if let Some(position) = self.pixel_position {
            self.anchor = Anchor::at(position, &self.heights);
        }
        let mut anchor = self.anchor;
        if let Some(y) = pending.jump {
            anchor = Anchor::at(y, &self.heights);
        }
        let page = f64::from(pending.pages) * (viewport - 40.).max(40.);
        anchor = anchor.moved(pending.delta + page, &self.heights);
        if let Some(block) = pending.reveal {
            anchor = anchor.revealing(block, viewport, &self.heights);
        }
        self.anchor = anchor.clamped(viewport, &self.heights);
        if self.pixel_position.is_some() {
            self.pixel_position = Some(self.anchor.content_y(&self.heights));
        }
        self.anchor
    }
}

/// The scrollbar's view of the state.
#[derive(Clone)]
pub(crate) struct Handle(pub(crate) Rc<RefCell<State>>);

impl ScrollbarHandle for Handle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().viewport
    }

    fn offset(&self) -> Point<Pixels> {
        let state = self.0.borrow();
        let y = state
            .pending
            .jump
            .unwrap_or_else(|| state.anchor.content_y(&state.heights));
        point(px(0.), px(-y as f32))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.0.borrow_mut().pending.jump = Some(-f64::from(f32::from(offset.y)));
    }

    fn content_size(&self) -> Size<Pixels> {
        let state = self.0.borrow();
        size(state.viewport.size.width, px(state.heights.total() as f32))
    }
}

pub(crate) struct SpikeView {
    pub(crate) handle: Handle,
    pub(crate) focus: FocusHandle,
}

impl SpikeView {
    pub(crate) fn new(blocks: usize, control: bool, cx: &mut Context<Self>) -> Self {
        Self {
            handle: Handle(Rc::new(RefCell::new(State::new(blocks, control)))),
            focus: cx.focus_handle(),
        }
    }

    pub(crate) fn from_env(cx: &mut App) -> Option<Entity<Self>> {
        let mode = std::env::var("JADE_SPIKE_SCROLL").ok()?;
        let start = std::time::Instant::now();
        let view = cx.new(|cx| Self::new(BLOCKS, mode == "pixels", cx));
        eprintln!(
            "[spike] {BLOCKS} blocks, {mode}, built in {:?}",
            start.elapsed()
        );
        Some(view)
    }
}

impl Render for SpikeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::scroll::Scrollbar;
        // The scrollbar comes after the element, so its prepaint sees this
        // frame's anchor and heights.
        div()
            .relative()
            .size_full()
            .child(ScrollElement::new(cx.entity()))
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&self.handle)
                        .id("spike-scrollbar")
                        .viewport_from_layout(),
                ),
            )
    }
}

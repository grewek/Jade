//! Headless evidence for #135.

use std::time::Instant;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Render, ScrollDelta, ScrollWheelEvent, Styled, TestAppContext,
    VisualTestContext, Window, WindowOptions, div, point, px, size,
};

use super::anchor::Anchor;
use super::element::{LAST, Snapshot};
use super::heights::HeightMap;
use super::{Random, SpikeView, State};

const VIEWPORT: f64 = 800.;

fn random_map(len: usize, random: &mut Random) -> HeightMap {
    HeightMap::new((0..len).map(|_| random.height()).collect())
}

#[test]
fn height_map_matches_a_plain_sum() {
    let mut random = Random::new(1);
    let mut map = random_map(1_000, &mut random);
    let mut plain: Vec<f64> = (0..1_000).map(|i| map.height(i)).collect();
    for _ in 0..5_000 {
        let block = random.below(1_000) as usize;
        let height = random.height();
        map.set(block, height);
        plain[block] = height;
        let probe = random.below(1_000) as usize;
        let before: f64 = plain[..probe].iter().sum();
        assert!((map.before(probe) - before).abs() < 1e-6);
        let (found, offset) = map.find(before + plain[probe] / 2.);
        assert_eq!(found, probe);
        assert!((offset - plain[probe] / 2.).abs() < 1e-6);
    }
    assert_eq!(map.find(-5.).0, 0);
    assert_eq!(map.find(1e12), (999, plain[999]));
}

/// Run with `--release` for the numbers.
#[test]
fn height_map_costs_and_drift() {
    let mut random = Random::new(2);
    let values: Vec<f64> = (0..100_000).map(|_| random.height()).collect();
    let start = Instant::now();
    let mut map = HeightMap::new(values.clone());
    println!("build 100k: {:?}", start.elapsed());
    let mut rebuilds: Vec<_> = (0..50)
        .map(|_| {
            // What inserting or removing a block costs with a Fenwick tree.
            let mut next = values.clone();
            let start = Instant::now();
            next.insert(50_000, 30.);
            std::hint::black_box(HeightMap::new(next));
            start.elapsed()
        })
        .collect();
    rebuilds.sort();
    println!("insert one block + rebuild, median: {:?}", rebuilds[25]);
    let start = Instant::now();
    for _ in 0..10_000_000 {
        let block = random.below(100_000) as usize;
        let height = random.height();
        map.set(block, height);
    }
    println!(
        "10M updates: {:?}; total {:.0} px, drift {:e} px",
        start.elapsed(),
        map.total(),
        map.drift()
    );
    assert!(map.drift() < 0.01);
    let total = map.total();
    let start = Instant::now();
    let mut sink = 0;
    for i in 0..1_000_000_u32 {
        sink += map.find(total * f64::from(i) / 1e6).0;
    }
    println!("1M finds: {:?} ({sink})", start.elapsed());
    // Why the map is f64: the next f32 after the document's height.
    let end = total as f32;
    println!(
        "f32 step at {end:.0} px: {} px",
        f32::from_bits(end.to_bits() + 1) - end
    );
}

/// The y of `block`'s top, relative to the viewport's top.
fn screen_y(block: usize, anchor: Anchor, heights: &HeightMap) -> f64 {
    let between = |from: usize, to: usize| (from..to).map(|i| heights.height(i)).sum::<f64>();
    if block >= anchor.block {
        between(anchor.block, block) - anchor.offset
    } else {
        -between(block, anchor.block) - anchor.offset
    }
}

/// 20,000 frames of scrolling while blocks above are remeasured: how far the
/// previous top block ends up from where scrolling alone puts it.
fn worst_jump(control: bool) -> (f64, u32) {
    let mut state = State::new(100_000, control);
    state.pending.jump = Some(state.heights.total() / 2.);
    state.settle(VIEWPORT);
    if control {
        state.pixel_position = Some(state.anchor.content_y(&state.heights));
    }
    let mut random = Random::new(3);
    let (mut worst, mut jumps) = (0_f64, 0);
    for step in 0..20_000 {
        let tracked = state.anchor;
        state.remeasure_above();
        // Rest, a touchpad's fraction of a pixel, a wheel notch, a fling.
        let delta = match step % 4 {
            0 => 0.,
            1 => (random.below(100) as f64 - 50.) / 7.,
            2 => (random.below(3) as f64 - 1.) * 72.,
            _ => (random.below(2_000) as f64 - 1_000.) * 1.3,
        };
        state.pending.delta = delta;
        if let Some(position) = state.pixel_position.as_mut() {
            *position += delta;
            state.pending.delta = 0.;
        }
        let anchor = state.settle(VIEWPORT);
        let error =
            (screen_y(tracked.block, anchor, &state.heights) - (-tracked.offset - delta)).abs();
        worst = worst.max(error);
        jumps += u32::from(error > 0.001);
    }
    (worst, jumps)
}

#[test]
fn content_stays_put_under_the_anchor() {
    let (worst, jumps) = worst_jump(false);
    println!("anchor: worst error {worst:e} px, frames off by > 0.001 px: {jumps}");
    assert_eq!(jumps, 0);
}

#[test]
fn control_a_pixel_offset_jumps() {
    let (worst, jumps) = worst_jump(true);
    println!("pixel offset: worst error {worst:.0} px, frames off: {jumps} of 20000");
    assert!(jumps > 10_000, "the check must be able to see a jump");
}

#[test]
fn ends_shrinking_and_revealing() {
    let map = HeightMap::new(vec![100.; 50]);
    let at = |block, offset| Anchor { block, offset };
    // Both ends stop.
    assert_eq!(at(0, 0.).moved(-1e9, &map).clamped(300., &map), at(0, 0.));
    assert_eq!(at(0, 0.).moved(1e9, &map).clamped(300., &map), at(47, 0.));
    // There and back is exact.
    let there = at(10, 30.).moved(1234.5, &map);
    assert_eq!(there.moved(-1234.5, &map), at(10, 30.));
    // A document shorter than the viewport stays at the top.
    let short = HeightMap::new(vec![50.; 2]);
    assert_eq!(at(1, 10.).clamped(300., &short), at(0, 0.));
    // The anchor block shrinks below the offset: the next block moves up to
    // the viewport's top. Content has to move here.
    let mut shrunk = HeightMap::new(vec![100.; 50]);
    shrunk.set(10, 20.);
    assert_eq!(at(10, 80.).clamped(300., &shrunk), at(11, 0.));
    // Reveal: nothing if visible, else the least movement.
    assert_eq!(at(10, 0.).revealing(11, 300., &map), at(10, 0.));
    assert_eq!(at(10, 0.).revealing(20, 300., &map), at(18, 0.));
    assert_eq!(at(10, 0.).revealing(5, 300., &map), at(5, 0.));
}

struct Host {
    view: Entity<SpikeView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

struct Opened {
    window: AnyWindowHandle,
    view: Entity<SpikeView>,
}

fn open(cx: &mut TestAppContext) -> Opened {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let opened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| Host {
                view: cx.new(|cx| SpikeView::new(100_000, false, cx)),
            })
        })
    });
    let Ok((window, host)) = opened else {
        panic!("cannot open a window");
    };
    let view = cx.update(|cx| host.read(cx).view.clone());
    let opened = Opened { window, view };
    VisualTestContext::from_window(window, cx).simulate_resize(size(px(900.), px(800.)));
    frame(cx, &opened);
    // Focus by clicking into the document.
    let mut visual = VisualTestContext::from_window(window, cx);
    visual.simulate_click(point(px(300.), px(300.)), Modifiers::default());
    frame(cx, &opened);
    opened
}

fn frame(cx: &mut TestAppContext, opened: &Opened) -> std::rc::Rc<Snapshot> {
    cx.update(|cx| opened.view.update(cx, |_, cx| cx.notify()));
    let done = cx.update_window(opened.window, |_, window, cx| window.render_frame(cx));
    assert!(done.is_ok(), "the window is gone");
    let Some(snapshot) = LAST.with(|last| last.borrow().clone()) else {
        panic!("nothing was painted");
    };
    snapshot
}

/// `(block, top, height)` of what is on screen, as bits.
fn positions(snapshot: &Snapshot) -> Vec<(usize, u32, u32)> {
    snapshot
        .blocks
        .iter()
        .map(|d| {
            let top = f32::from(d.bounds.origin.y).to_bits();
            (d.block, top, f32::from(d.bounds.size.height).to_bits())
        })
        .collect()
}

fn wheel(cx: &mut TestAppContext, opened: &Opened, delta: ScrollDelta) -> std::rc::Rc<Snapshot> {
    let mut visual = VisualTestContext::from_window(opened.window, cx);
    visual.simulate_event(ScrollWheelEvent {
        position: point(px(300.), px(300.)),
        delta,
        ..Default::default()
    });
    frame(cx, opened)
}

fn top_of(snapshot: &Snapshot, block: usize) -> Pixels {
    let drawn = snapshot.blocks.iter().find(|d| d.block == block);
    let Some(drawn) = drawn else {
        panic!("block {block} is not on screen");
    };
    drawn.bounds.origin.y
}

#[gpui_kit::test]
fn drawn_blocks_do_not_move_while_heights_above_change(cx: &mut TestAppContext) {
    let opened = open(cx);
    cx.simulate_keystrokes(opened.window, "m");
    let still = positions(&frame(cx, &opened));
    let handle = cx.update(|cx| opened.view.read(cx).handle.clone());
    let above = || {
        let state = handle.0.borrow();
        state.heights.before(state.anchor.block)
    };
    handle.0.borrow_mut().remeasure = true;
    let (mut previous, mut changed) = (above(), 0.);
    for _ in 0..500 {
        let now = frame(cx, &opened);
        assert_eq!(positions(&now), still, "something moved");
        changed += (above() - previous).abs();
        previous = above();
    }
    println!(
        "500 frames: content above changed by {changed:.0} px in all, {} blocks bit-identical",
        still.len()
    );
    assert!(changed > 100_000., "the heights above did change");
}

#[gpui_kit::test]
fn wheel_lines_pixels_keys_and_scrollbar_move_the_anchor(cx: &mut TestAppContext) {
    let opened = open(cx);
    cx.simulate_keystrokes(opened.window, "m");
    let mut shot = frame(cx, &opened);
    let handle = cx.update(|cx| opened.view.read(cx).handle.clone());
    handle.0.borrow_mut().remeasure = true;
    let line_height = cx
        .update_window(opened.window, |_, window, _| window.line_height())
        .ok();
    // A touchpad sends fractions of pixels, a wheel sends lines.
    for delta in [
        ScrollDelta::Pixels(point(px(0.), px(-0.75))),
        ScrollDelta::Pixels(point(px(0.), px(-13.5))),
        ScrollDelta::Lines(point(0., -3.)),
    ] {
        let tracked = shot.blocks[1].block;
        let before = top_of(&shot, tracked);
        shot = wheel(cx, &opened, delta);
        let moved = f32::from(top_of(&shot, tracked) - before);
        let wanted = line_height.map(|h| f32::from(delta.pixel_delta(h).y));
        println!("{delta:?}: moved {moved} px");
        assert!(wanted.is_some_and(|w| (moved - w).abs() < 0.001));
    }
    handle.0.borrow_mut().remeasure = false;

    cx.simulate_keystrokes(opened.window, "home");
    assert_eq!(frame(cx, &opened).anchor, Anchor::default());
    cx.simulate_keystrokes(opened.window, "pagedown");
    let paged = frame(cx, &opened).anchor;
    assert!((paged.content_y(&handle.0.borrow().heights) - 760.).abs() < 0.001);
    cx.simulate_keystrokes(opened.window, "pageup");
    assert!(frame(cx, &opened).anchor.offset.abs() < 0.001);
    cx.simulate_keystrokes(opened.window, "end");
    let end = frame(cx, &opened);
    let Some(last) = end.blocks.last() else {
        panic!("nothing drawn at the end");
    };
    assert_eq!(last.block, 99_999);
    assert!((f32::from(last.bounds.bottom()) - 800.).abs() < 0.01);
    cx.simulate_keystrokes(opened.window, "g");
    assert_eq!(frame(cx, &opened).anchor.block, 25_000);

    // Drag the gpui-kit scrollbar's thumb from a quarter to the bottom.
    cx.simulate_keystrokes(opened.window, "home");
    frame(cx, &opened);
    let mut visual = VisualTestContext::from_window(opened.window, cx);
    let grab = point(px(892.), px(20.));
    visual.simulate_mouse_move(grab, None, Modifiers::default());
    frame(cx, &opened);
    let mut visual = VisualTestContext::from_window(opened.window, cx);
    visual.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    for (y, wanted) in [(396., 0.5), (900., 1.0), (20., 0.0)] {
        frame(cx, &opened);
        let to = point(px(892.), px(y));
        let mut visual = VisualTestContext::from_window(opened.window, cx);
        visual.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        let shot = frame(cx, &opened);
        let state = handle.0.borrow();
        let max = state.heights.total() - 800.;
        let share = shot.anchor.content_y(&state.heights) / max;
        println!("thumb dragged to y={y}: at {:.4} of the document", share);
        assert!((share - wanted).abs() < 0.01);
    }
}

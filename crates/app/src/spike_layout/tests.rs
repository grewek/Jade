//! Headless evidence for #134's three questions, on GPUI's test platform.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Context, Entity, EntityInputHandler, InteractiveElement,
    IntoElement, Modifiers, ParentElement, Render, StatefulInteractiveElement, Styled,
    TestAppContext, VisualTestContext, Window, WindowOptions, div, point, px, size,
};

use super::trace::FRAMES;
use super::{Mode, SpikeView};

struct Opened {
    window: AnyWindowHandle,
    view: Entity<SpikeView>,
}

fn open(cx: &mut TestAppContext, mode: Mode) -> Opened {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        cx.bind_keys(super::key_bindings());
    });
    let text: String = (0..400)
        .map(|i| {
            format!(
                "paragraph {i} {}\n",
                "with some words to wrap ".repeat(i % 9)
            )
        })
        .collect();
    let opened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
            cx.new(|cx| Host {
                view: cx.new(|cx| SpikeView::new(&text, mode, cx)),
            })
        })
    });
    let Ok((window, host)) = opened else {
        panic!("cannot open a window");
    };
    let view = cx.update(|cx| host.read(cx).view.clone());
    let opened = Opened { window, view };
    frame(cx, &opened);
    opened
}

/// The same two hostings the app uses, plus a sidebar that can be toggled.
struct Host {
    view: Entity<SpikeView>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = div().id("pane").flex_1().h_full().min_w_0();
        let pane = if self.view.read(cx).mode == Mode::Measured {
            pane.overflow_y_scroll()
        } else {
            pane.overflow_hidden()
        };
        let sidebar = SIDEBAR.with(std::cell::Cell::get);
        div()
            .flex()
            .size_full()
            .child(div().w(px(sidebar)).h_full().flex_none())
            .child(pane.child(self.view.clone()))
    }
}

thread_local! {
    static SIDEBAR: std::cell::Cell<f32> = const { std::cell::Cell::new(240.) };
}

fn frame(cx: &mut TestAppContext, opened: &Opened) {
    let done = cx.update_window(opened.window, |_, window, cx| window.render_frame(cx));
    assert!(done.is_ok(), "the window is gone");
}

fn field(line: &str, name: &str) -> f32 {
    let start = line.find(name).map(|at| at + name.len() + 1);
    let value = start.and_then(|at| line[at..].split(' ').next()?.parse().ok());
    let Some(value) = value else {
        panic!("no field {name} in {line}");
    };
    value
}

/// Q1 and Q2: one frame after each width change, every phase has the new width.
/// Paint's state hash check (a debug assertion) runs in every frame here.
fn widths_follow_in_the_same_frame(mode: Mode, cx: &mut TestAppContext) {
    let opened = open(cx, mode);
    let steps = [
        (1000., 240.),
        (700., 240.),
        (700., 0.),
        (1400., 0.),
        (900., 240.),
    ];
    for (window_width, sidebar) in steps {
        FRAMES.with(|frames| frames.borrow_mut().clear());
        SIDEBAR.with(|s| s.set(sidebar));
        VisualTestContext::from_window(opened.window, cx)
            .simulate_resize(size(px(window_width), px(600.)));
        cx.run_until_parked();
        cx.update(|cx| opened.view.update(cx, |_, cx| cx.notify()));
        frame(cx, &opened);
        let frames = FRAMES.with(|frames| frames.borrow().clone());
        let expected = window_width - sidebar;
        println!(
            "{mode:?} window={window_width} sidebar={sidebar}, {} frames:",
            frames.len()
        );
        for line in &frames {
            println!("  {line}");
        }
        // The host re-reads the sidebar only when it renders, so frames drawn
        // by the resize itself may still have the old sidebar. What must hold
        // in every frame is that the phases agree.
        for line in &frames {
            let width = field(line, "prepaint_w");
            assert_eq!(field(line, "shaped_w"), width, "{line}");
            assert_eq!(field(line, "paint_w"), width, "{line}");
            if mode == Mode::Measured {
                let used = line
                    .split("used=[")
                    .nth(1)
                    .and_then(|s| s.split(']').next());
                // Empty when GPUI reused the layout; then nothing was measured.
                if let Some(last) = used.and_then(|s| s.split(' ').next_back()?.parse::<f32>().ok())
                {
                    assert_eq!(last, width, "height from another width: {line}");
                }
            }
        }
        let Some(line) = frames.last() else {
            panic!("no frame was drawn");
        };
        assert_eq!(field(line, "prepaint_w"), expected, "{line}");
        if mode == Mode::Measured {
            let height = cx.update(|cx| {
                let view = opened.view.read(cx);
                view.snapshot.as_ref().map(|s| s.bounds.size.height)
            });
            assert_eq!(
                height.map(f32::from).map(f32::round),
                Some(field(&line, "h_measure"))
            );
        } else {
            assert!(
                line.contains("measure=[]"),
                "fill mode never measures: {line}"
            );
        }
    }
}

#[gpui_kit::test]
fn measured_mode_uses_this_frames_width(cx: &mut TestAppContext) {
    widths_follow_in_the_same_frame(Mode::Measured, cx);
}

#[gpui_kit::test]
fn fill_mode_uses_this_frames_width(cx: &mut TestAppContext) {
    widths_follow_in_the_same_frame(Mode::Fill, cx);
}

#[gpui_kit::test]
#[should_panic(expected = "paint changed layout state")]
fn the_paint_check_catches_a_mutation(cx: &mut TestAppContext) {
    let opened = open(cx, Mode::Fill);
    cx.update(|cx| {
        opened.view.update(cx, |view, cx| {
            view.paint_mutates = true;
            cx.notify();
        })
    });
    frame(cx, &opened);
}

/// Q3: focus, platform text input, key events, key context and click.
#[gpui_kit::test]
fn input_focus_and_key_context_work(cx: &mut TestAppContext) {
    let opened = open(cx, Mode::Fill);
    let first_line =
        |cx: &mut TestAppContext| cx.update(|cx| opened.view.read(cx).lines[0].clone());
    let original = first_line(cx);

    cx.simulate_input(opened.window, "x");
    assert_eq!(first_line(cx), original, "unfocused: nothing is typed");

    let mut visual = VisualTestContext::from_window(opened.window, cx);
    visual.simulate_click(point(px(242.), px(10.)), Modifiers::default());
    visual.run_until_parked();
    frame(cx, &opened);
    let focused = cx.update_window(opened.window, |_, window, cx| {
        opened.view.read(cx).focus.is_focused(window)
    });
    assert_eq!(focused.ok(), Some(true), "a click focuses");
    assert_eq!(cx.update(|cx| opened.view.read(cx).cursor), (0, 0));

    cx.simulate_input(opened.window, "héllo ");
    assert_eq!(first_line(cx), format!("héllo {original}"));
    cx.simulate_keystrokes(opened.window, "backspace");
    assert_eq!(first_line(cx), format!("héllo{original}"));
    cx.simulate_keystrokes(opened.window, "tab");
    assert_eq!(first_line(cx), format!("héllo    {original}"));
    assert_eq!(cx.update(|cx| opened.view.read(cx).tabs_handled), 1);
    cx.simulate_keystrokes(opened.window, "enter");
    assert_eq!(first_line(cx), "héllo    ");
    assert_eq!(cx.update(|cx| opened.view.read(cx).cursor), (1, 0));
}

/// Q3, composition: the calls the Wayland backend makes for a dead key.
/// This drives the handler directly, not a real input method.
#[gpui_kit::test]
fn composition_replaces_its_preview(cx: &mut TestAppContext) {
    let opened = open(cx, Mode::Fill);
    let view = opened.view.clone();
    let done = cx.update_window(opened.window, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.lines[0].clear();
            view.cursor = (0, 0);
            view.replace_and_mark_text_in_range(None, "´", None, window, cx);
            assert_eq!(view.marked_text_range(window, cx), Some(0..1));
            view.replace_text_in_range(None, "é", window, cx);
            assert_eq!(view.lines[0], "é");
            assert_eq!(view.marked, None);
        });
    });
    assert!(done.is_ok());
    frame(cx, &opened);
    let area = cx.update_window(opened.window, |_, window, cx| {
        opened.view.update(cx, |view, cx| {
            view.bounds_for_range(1..1, Bounds::default(), window, cx)
        })
    });
    let area = area.ok().flatten();
    println!("IME candidate area after composing: {area:?}");
    assert!(area.is_some_and(|a| a.origin.x > px(240.)));
}

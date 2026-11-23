//! A row holding a tall formula is taller, and painting, hit-testing and the
//! arrow keys agree on it (#15).

use super::super::Editor;
use super::LineLayout;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Entity, Keystroke, Pixels, TestAppContext, WindowOptions, point,
    px,
};

const TALL: &str = "$\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}$";
const SMALL: &str = "$x^2$";

fn open(cx: &mut TestAppContext, text: &'static str) -> (AnyWindowHandle, Entity<Editor>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        cx.bind_keys(crate::text_editor::key_bindings());
    });
    let opened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, move |_window, cx| {
            cx.new(|cx| Editor::new(text, cx))
        })
        .expect("open test window")
    });
    // The caret stays on line 0, so the formula on line 1 is drawn, not raw.
    opened
        .1
        .update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
    opened
}

/// The editor after two frames (row counts are made exact while painting),
/// with its layout and the editor's row height.
fn settle(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    editor: &Entity<Editor>,
) -> (LineLayout, Pixels) {
    let line_height = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.line_height()
        })
        .unwrap();
    let view = editor
        .read_with(cx, |editor, _| editor.last_view.clone())
        .unwrap();
    (view.layout.clone(), line_height)
}

fn note(formula: &str) -> &'static str {
    Box::leak(format!("intro\nsee {formula} here\nlast line").into_boxed_str())
}

#[gpui_kit::test]
fn a_tall_formula_makes_its_line_taller(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, note(TALL));
    let (layout, lh) = settle(cx, window, &editor);
    let rows = (layout.height(1) / lh).round();
    assert!(rows >= 2., "the matrix line is {rows} rows");
    assert!(
        (layout.height(1) / lh - rows).abs() < 1e-3,
        "whole rows only"
    );
    assert_eq!(layout.height(0), lh);
    assert_eq!(layout.height(2), lh);
}

#[gpui_kit::test]
fn a_small_formula_keeps_the_normal_row(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, note(SMALL));
    let (layout, lh) = settle(cx, window, &editor);
    assert_eq!(layout.height(1), lh);
}

#[gpui_kit::test]
fn clicks_land_on_the_line_under_them_either_side_of_a_tall_row(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, _) = settle(cx, window, &editor);
    let (top, bottom) = (layout.cumulative_at(1), layout.cumulative_at(2));
    // A click moves the caret onto its line, which then draws raw and shrinks
    // back to one row: put the caret back before each probe.
    let line = |at: Pixels, cx: &mut TestAppContext| {
        editor.update(cx, |editor, _| editor.buffer_mut().set_cursor(0, false));
        settle(cx, window, &editor);
        cx.update_window(window, |_, window, cx| {
            window.click_at("editor", point(px(1.), at), cx);
        })
        .unwrap();
        let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
        text[..cursor].matches('\n').count()
    };
    assert_eq!(line(top + px(2.), cx), 1, "just inside the tall row");
    assert_eq!(line(bottom - px(2.), cx), 1, "just above the next line");
    assert_eq!(line(bottom + px(2.), cx), 2, "just inside the next line");
}

#[gpui_kit::test]
fn arrow_up_from_the_line_below_goes_to_the_tall_line(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, lh) = settle(cx, window, &editor);
    let below = layout.cumulative_at(2) + lh / 2.;
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), below), cx);
        window.render_frame(cx);
        window.dispatch_keystroke(Keystroke::parse("up").unwrap(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
    assert_eq!(text[..cursor].matches('\n').count(), 1);
}

#[gpui_kit::test]
fn arrow_down_from_the_tall_line_goes_to_the_line_below(cx: &mut TestAppContext) {
    let text = note(TALL);
    let (window, editor) = open(cx, text);
    let (layout, _) = settle(cx, window, &editor);
    let inside = layout.cumulative_at(1) + px(2.);
    cx.update_window(window, |_, window, cx| {
        window.click_at("editor", point(px(1.), inside), cx);
        window.render_frame(cx);
        window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());
    assert_eq!(text[..cursor].matches('\n').count(), 2);
}

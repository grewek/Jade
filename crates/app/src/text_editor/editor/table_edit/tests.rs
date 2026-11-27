//! A body row with more cells than the header. This rendering path draws no box
//! for the extra ones, so none of them may become the edited cell (#140: doing
//! so panicked while painting).

use super::super::Editor;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext, WindowOptions};

/// Two columns; the middle row has a third cell.
const TABLE: &str = "| a | b |\n|---|---|\n| c | d | e |\n| f | g |\n\nend";

fn at(cell: char) -> usize {
    TABLE.find(cell).unwrap()
}

/// A focused editor over [`TABLE`] with its caret at byte `cursor`, painted.
fn open_at(cx: &mut TestAppContext, cursor: usize) -> (AnyWindowHandle, Entity<Editor>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, editor) = cx.update(|cx| {
        // Tab only reaches the editor through its own binding.
        cx.bind_keys(crate::text_editor::key_bindings());
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new(TABLE, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        window.focus(&editor.focus_handle(cx), cx);
        editor.update(cx, |e, _| e.buffer_mut().set_cursor(cursor, false));
        window.render_frame(cx);
    })
    .unwrap();
    (window, editor)
}

/// Presses `key`, paints, and returns the caret. The text must not change.
fn press(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    editor: &Entity<Editor>,
    key: &str,
) -> usize {
    cx.update_window(window, |_, window, cx| {
        window.press(key, cx);
        window.render_frame(cx);
    })
    .unwrap();
    editor.read_with(cx, |e, _| {
        assert_eq!(e.buffer().text(), TABLE);
        e.buffer().cursor()
    })
}

#[gpui_kit::test]
fn a_caret_in_a_cell_that_is_not_drawn_goes_to_the_last_one_that_is(cx: &mut TestAppContext) {
    let (_, editor) = open_at(cx, at('e') + 1);
    editor.read_with(cx, |e, _| {
        assert_eq!(e.buffer().text(), TABLE);
        assert_eq!(e.buffer().cursor(), at('d') + 1);
    });
}

#[gpui_kit::test]
fn right_and_left_step_over_the_cell_that_is_not_drawn(cx: &mut TestAppContext) {
    let (window, editor) = open_at(cx, at('d') + 1);
    assert_eq!(press(cx, window, &editor, "right"), at('f'));
    assert_eq!(press(cx, window, &editor, "left"), at('d') + 1);
}

#[gpui_kit::test]
fn tab_and_vertical_movement_stay_in_drawn_cells(cx: &mut TestAppContext) {
    let (window, editor) = open_at(cx, at('d'));
    assert_eq!(press(cx, window, &editor, "tab"), at('f') + 1);
    assert_eq!(press(cx, window, &editor, "shift-tab"), at('d') + 1);
    assert_eq!(press(cx, window, &editor, "down"), at('g') + 1);
    let up = press(cx, window, &editor, "up");
    assert!((at('d')..=at('d') + 1).contains(&up), "{up}");
}

//! Table crashes the fuzzer found (#140), each an index that the table on
//! screen did not have.

use super::super::Editor;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext, WindowOptions};

/// A body row with more cells than the header. This rendering path draws no
/// box for the extra ones, so none of them may become the edited cell.
///
/// Two columns; the middle row has a third cell.
const TABLE: &str = "| a | b |\n|---|---|\n| c | d | e |\n| f | g |\n\nend";

fn at(cell: char) -> usize {
    TABLE.find(cell).unwrap()
}

/// A focused editor over [`TABLE`] with its caret at byte `cursor`, painted.
fn open_at(cx: &mut TestAppContext, cursor: usize) -> (AnyWindowHandle, Entity<Editor>) {
    open(cx, TABLE, cursor)
}

/// [`open_at`] over any `text`.
fn open(
    cx: &mut TestAppContext,
    text: &'static str,
    cursor: usize,
) -> (AnyWindowHandle, Entity<Editor>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, editor) = cx.update(|cx| {
        // Tab only reaches the editor through its own binding.
        cx.bind_keys(crate::text_editor::key_bindings());
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new(text, cx))
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

// What the pointer is over or dragging is remembered as row and column numbers.
// An edit can take those rows away before the pointer moves again.

const ROWS: &str = "| a | b |\n|---|---|\n| c | d |\n| e | f |\n\nend";

/// A mouse event `x` pixels right of and `rows` row heights below the corner
/// of a table that starts the note, followed by a frame.
fn mouse(cx: &mut TestAppContext, window: AnyWindowHandle, kind: &str, (x, rows): (f32, f32)) {
    use gpui_kit::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput};
    cx.update_window(window, |_, window, cx| {
        let corner = window.find("editor").bounds().origin;
        let position = corner + gpui_kit::point(gpui_kit::px(x), window.line_height() * rows);
        let (button, modifiers) = (MouseButton::Left, Default::default());
        let event = match kind {
            "down" => PlatformInput::MouseDown(MouseDownEvent {
                button,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            "up" => PlatformInput::MouseUp(MouseUpEvent {
                button,
                position,
                modifiers,
                click_count: 1,
            }),
            _ => PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: (kind == "drag").then_some(button),
                modifiers,
            }),
        };
        window.dispatch_event(event, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn key(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| {
        window.press(key, cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn text_of(cx: &mut TestAppContext, editor: &Entity<Editor>) -> String {
    editor.read_with(cx, |e, _| e.buffer().text().to_string())
}

/// [`ROWS`] after Tab in its last cell added a fourth row, so that undo takes
/// it away again.
fn open_with_an_added_row(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Editor>) {
    let (window, editor) = open(cx, ROWS, ROWS.find('f').unwrap());
    key(cx, window, "tab");
    assert_ne!(text_of(cx, &editor), ROWS);
    (window, editor)
}

#[gpui_kit::test]
fn controls_shown_for_a_row_that_undo_removes_do_not_crash(cx: &mut TestAppContext) {
    let (window, editor) = open_with_an_added_row(cx);
    // At the bottom edge of the fourth row.
    mouse(cx, window, "move", (10.0, 3.9));
    assert!(editor.read_with(cx, |e, _| e.table_hover.is_some()));
    key(cx, window, "ctrl-z");
    assert_eq!(text_of(cx, &editor), ROWS);
}

#[gpui_kit::test]
fn undo_during_a_drag_of_the_row_it_removes_ends_the_drag(cx: &mut TestAppContext) {
    let (window, editor) = open_with_an_added_row(cx);
    // The grip on the left edge, halfway down the fourth row.
    mouse(cx, window, "down", (0.0, 3.5));
    assert!(editor.read_with(cx, |e, _| e.table_drag.is_some()));
    key(cx, window, "ctrl-z");
    mouse(cx, window, "up", (0.0, 1.5));
    assert_eq!(text_of(cx, &editor), ROWS);
}

#[gpui_kit::test]
fn a_drag_released_after_its_table_is_gone_does_nothing(cx: &mut TestAppContext) {
    let (window, editor) = open(cx, "end", 0);
    cx.update(|cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "| a | b |\n|---|---|\n| c | d |\n\n".into(),
        ))
    });
    key(cx, window, "ctrl-v");
    mouse(cx, window, "down", (0.0, 1.5));
    assert!(editor.read_with(cx, |e, _| e.table_drag.is_some()));
    key(cx, window, "ctrl-z");
    mouse(cx, window, "up", (0.0, 1.5));
    assert_eq!(text_of(cx, &editor), "end");
}

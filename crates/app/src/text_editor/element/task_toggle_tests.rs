//! Ticking a task in a note that is not focused yet must not scroll the view
//! to the caret at the top.

use super::super::Editor;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};

#[gpui_kit::test]
fn the_first_checkbox_click_does_not_send_the_view_to_the_caret(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
    });
    let (window, editor) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_w, cx| {
            cx.new(|cx| Editor::new("intro\n\n- [ ] one\n- [ ] last\n", cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        let at = "intro\n\n- [ ] one\n- [ ] last\n".find("last").unwrap() - 4;
        editor.update(cx, |editor, cx| {
            assert_eq!(editor.last_autoscrolled_cursor.get(), None);
            editor.toggle_checkbox(at..at + 4, window, cx);
            assert!(editor.buffer().text().contains("- [x] last"));
            assert_eq!(
                editor.last_autoscrolled_cursor.get(),
                Some(editor.buffer().cursor()),
                "the next paint must treat the caret as already seen"
            );
        });
    })
    .unwrap();
}

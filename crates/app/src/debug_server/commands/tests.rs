//! Commands run against a whole app window.

use std::path::Path;

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use jade_debug_protocol::{Answer, Command, EditorState, Query};

use super::execute;
use crate::JadeApp;

/// An app on `vault` with the sidebar painted and no note open.
fn open_app(cx: &mut TestAppContext, vault: &Path) -> (AnyWindowHandle, Entity<JadeApp>) {
    let vault = vault.to_path_buf();
    let (window, app) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::ThemeEngine::install(None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| JadeApp::new(vault, window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (window, app)
}

fn run(cx: &mut TestAppContext, app: &Entity<JadeApp>, command: Command) -> Answer {
    let app = app.downgrade();
    cx.update(|cx| execute(command, &app, cx))
}

fn editor_state(cx: &mut TestAppContext, app: &Entity<JadeApp>) -> EditorState {
    let query = Command::Query {
        what: Query::EditorState,
    };
    match run(cx, app, query) {
        Answer::EditorState(state) => state,
        other => panic!("no editor state: {other:?}"),
    }
}

#[gpui_kit::test]
fn focus_editor_gives_the_focus_and_leaves_the_note_alone(cx: &mut TestAppContext) {
    let vault = tempfile::tempdir().unwrap();
    std::fs::write(vault.path().join("Note.md"), "one\n\ntwo").unwrap();
    let (window, app) = open_app(cx, vault.path());
    assert!(matches!(
        run(cx, &app, Command::FocusEditor),
        Answer::Error { message } if message == "no note is open"
    ));

    cx.update_window(window, |_, window, cx| {
        window.click("Note", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let before = editor_state(cx, &app);
    assert!(!before.focused, "the click was in the sidebar");

    assert_eq!(run(cx, &app, Command::FocusEditor), Answer::Ok);
    let after = editor_state(cx, &app);
    assert!(after.focused);
    assert_eq!((after.cursor, after.len), (before.cursor, before.len));
}

#[gpui_kit::test]
fn focus_editor_refuses_to_reach_behind_a_dialog(cx: &mut TestAppContext) {
    let vault = tempfile::tempdir().unwrap();
    std::fs::write(vault.path().join("Note.md"), "text").unwrap();
    let (window, app) = open_app(cx, vault.path());
    cx.update_window(window, |_, window, cx| {
        window.click("Note", cx);
        window.open_dialog(cx, |dialog, _, _| dialog);
        window.render_frame(cx);
    })
    .unwrap();

    assert!(matches!(
        run(cx, &app, Command::FocusEditor),
        Answer::Error { message } if message == "a dialog is open"
    ));
    assert!(!editor_state(cx, &app).focused);
}

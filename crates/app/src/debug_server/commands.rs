//! Commands that must run on the GPUI main thread, with window access.

use gpui_kit::{App, Focusable, WeakEntity};
use jade_debug_protocol::{Answer, Command, EditorState, Event, Query, Topic};

use super::input;
use crate::JadeApp;

fn error(message: impl Into<String>) -> Answer {
    Answer::Error {
        message: message.into(),
    }
}

pub(super) fn execute(command: Command, app: &WeakEntity<JadeApp>, cx: &mut App) -> Answer {
    match command {
        Command::Query {
            what: Query::WindowSize,
        } => window_size(cx),
        Command::Query {
            what: Query::EditorState,
        } => editor_state(app, cx),
        command if input::is_input(&command) => inject(&command, cx),
        other => error(format!("not implemented yet: {other:?}")),
    }
}

fn first_window(cx: &App) -> Option<gpui_kit::AnyWindowHandle> {
    cx.windows().into_iter().next()
}

fn window_size(cx: &mut App) -> Answer {
    let Some(handle) = first_window(cx) else {
        return error("no window is open");
    };
    match handle.update(cx, |_, window, _| window.viewport_size()) {
        Ok(size) => Answer::WindowSize {
            width: size.width.into(),
            height: size.height.into(),
        },
        Err(err) => error(format!("window unavailable: {err}")),
    }
}

fn inject(command: &Command, cx: &mut App) -> Answer {
    let Some(handle) = first_window(cx) else {
        return error("no window is open");
    };
    if super::wants(Topic::Input) {
        super::emit(&Event::Input {
            description: format!("{command:?}"),
        });
    }
    match handle.update(cx, |_, window, cx| input::apply(command, window, cx)) {
        Ok(Ok(())) => Answer::Ok,
        Ok(Err(message)) => error(message),
        Err(err) => error(format!("window unavailable: {err}")),
    }
}

fn editor_state(app: &WeakEntity<JadeApp>, cx: &mut App) -> Answer {
    let (Some(app), Some(handle)) = (app.upgrade(), first_window(cx)) else {
        return error("the app window is gone");
    };
    let state = handle.update(cx, |_, window, cx| {
        let app = app.read(cx);
        let mut state = EditorState {
            note: app.selected_id.clone(),
            ..Default::default()
        };
        let Some(editor) = app.editor.as_ref() else {
            return state;
        };
        let editor = editor.read(cx);
        let buffer = editor.buffer();
        state.cursor = buffer.cursor();
        state.selection = buffer.selection().map(|r| (r.start, r.end));
        state.len = buffer.text().len();
        state.lines = buffer.text().bytes().filter(|&b| b == b'\n').count() + 1;
        state.scroll_y = editor.scroll_handle().offset().y.into();
        state.focused = editor.focus_handle(cx).is_focused(window);
        state
    });
    match state {
        Ok(state) => Answer::EditorState(state),
        Err(err) => error(format!("window unavailable: {err}")),
    }
}

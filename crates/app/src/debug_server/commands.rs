//! Commands that must run on the GPUI main thread, with window access.

use gpui_kit::App;
use jade_debug_protocol::{Answer, Command, Query};

fn error(message: impl Into<String>) -> Answer {
    Answer::Error {
        message: message.into(),
    }
}

pub(super) fn execute(command: Command, cx: &mut App) -> Answer {
    match command {
        Command::Query {
            what: Query::WindowSize,
        } => window_size(cx),
        other => error(format!("not implemented yet: {other:?}")),
    }
}

fn window_size(cx: &mut App) -> Answer {
    let Some(handle) = cx.windows().into_iter().next() else {
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

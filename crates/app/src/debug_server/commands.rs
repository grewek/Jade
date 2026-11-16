//! Commands that must run on the GPUI main thread, with window access.

use gpui_kit::App;
use jade_debug_protocol::{Answer, Command, Event, Query, Topic};

use super::input;

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

//! Commands that must run on the GPUI main thread, with window access.

use gpui_kit::component::WindowExt;
use gpui_kit::{App, Focusable, WeakEntity};
use jade_debug_protocol::{
    Answer, AppState, Command, EditorState, Event, MAX_TEXT_BYTES, Query, ThemeState, Topic,
};

use super::input;
use crate::JadeApp;
use crate::theme::{Kind, Role, ThemeEngine};

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
        Command::Query {
            what: Query::AppState,
        } => app_state(app, cx),
        Command::Query { what: Query::Theme } => theme_state(cx),
        Command::SetTheme { id } => match ThemeEngine::set(&id, cx) {
            Ok(()) => Answer::Ok,
            Err(messages) => error(messages.join("; ")),
        },
        Command::ReadEditorText { start, len } => read_text(app, start, len, cx),
        // The pump quits after the reply is sent.
        Command::Quit => Answer::Ok,
        Command::ListTargets => Answer::Targets {
            targets: super::targets::list(),
        },
        command if input::is_input(&command) => inject(&command, cx),
        other => error(format!("not implemented yet: {other:?}")),
    }
}

fn hex(color: gpui_kit::Hsla) -> String {
    let c = gpui_kit::Rgba::from(color);
    let byte = |v: f32| (v * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

fn theme_state(cx: &App) -> Answer {
    let engine = ThemeEngine::global(cx);
    let kind = match engine.kind() {
        Kind::Light => "light",
        Kind::Dark => "dark",
    };
    Answer::Theme(ThemeState {
        id: engine.active_id().to_owned(),
        kind: kind.into(),
        background: hex(engine.palette().color(Role::Background)),
        text: hex(engine.palette().color(Role::Text)),
        accent: hex(engine.palette().color(Role::Accent)),
    })
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
            note: app.selected_id().map(str::to_string),
            ..Default::default()
        };
        let Some(editor) = app.editor() else {
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

fn app_state(app: &WeakEntity<JadeApp>, cx: &mut App) -> Answer {
    let (Some(app), Some(handle)) = (app.upgrade(), first_window(cx)) else {
        return error("the app window is gone");
    };
    let state = handle.update(cx, |_, window, cx| {
        let dialog_open = window.has_active_dialog(cx);
        let app = app.read(cx);
        AppState {
            note: app.selected_id().map(str::to_string),
            tabs: app.tabs.iter().map(|tab| tab.note_id.clone()).collect(),
            active_tab: app.active_tab,
            words: app.editor().map_or(0, |e| e.read(cx).word_count()),
            chars: app.editor().map_or(0, |e| e.read(cx).char_count()),
            sidebar_tab: match app.sidebar_tab {
                crate::SidebarTab::Vault => "vault",
                crate::SidebarTab::Search => "search",
            }
            .into(),
            search_query: app.search_query.clone(),
            search_results: app.search_results.len(),
            sidebar_width: f32::from(app.sidebar_width),
            backlinks_width: f32::from(app.backlinks_width),
            dialog_open,
            error: app.error.clone(),
        }
    });
    match state {
        Ok(state) => Answer::AppState(state),
        Err(err) => error(format!("window unavailable: {err}")),
    }
}

/// A slice of the open note, cut on character boundaries.
fn read_text(app: &WeakEntity<JadeApp>, start: usize, len: usize, cx: &App) -> Answer {
    let Some(app) = app.upgrade() else {
        return error("the app window is gone");
    };
    let Some(editor) = app.read(cx).editor().cloned() else {
        return error("no note is open");
    };
    let text = editor.read(cx).buffer().text();
    let start = text.floor_char_boundary(start.min(text.len()));
    let end = text.floor_char_boundary(
        start
            .saturating_add(len.min(MAX_TEXT_BYTES))
            .min(text.len()),
    );
    Answer::Text {
        start,
        text: text[start..end].to_owned(),
        total_len: text.len(),
    }
}

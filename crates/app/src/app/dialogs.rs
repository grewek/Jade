//! The modal dialogs and palettes: rename, delete, create, settings, commands.

#[allow(unused_imports)]
use super::*;

/// One entry of the Settings theme dropdown; the value is the theme id.
#[derive(Clone)]
pub(super) struct ThemeOption {
    id: String,
    name: SharedString,
}

impl gpui_kit::component::select::SelectItem for ThemeOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

impl JadeApp {
    /// Opens a `Command` palette (gpui-component's own searchable action-
    /// list widget) hosted inside a plain `Dialog`, same `window.open_dialog`
    /// mechanism `open_rename_dialog`/`open_delete_dialog` already use below
    /// (which already renders above the whole window and has its own built-
    /// in close "x", so there's nothing extra to wire up for either of
    /// those). Only one item for now — confirming it (`on_confirm`, the
    /// palette's own Enter/click handler) closes this dialog and opens the
    /// (currently non-functional) Settings one; `Command` has no idea it's
    /// hosted in a `Dialog`, so closing it is explicit, not automatic.
    ///
    /// A **plain associated function**, not an action-handling `&mut self`
    /// method dispatched via `.on_action(cx.listener(...))` on some element
    /// — Ctrl+P must work even with *nothing* in the app focused yet (e.g.
    /// right after opening, or right after closing a dialog), and GPUI's
    /// action dispatch only walks a path from the currently-*focused* node
    /// up to the tree root; with no focus at all, that path collapses to a
    /// single handler-less root node, so an element-attached listener a
    /// level below would simply never be reached. `cx.on_action::<OpenCommandPalette>(...)`
    /// (registered once in `main()`, see below) sidesteps this entirely —
    /// global action listeners run unconditionally, independent of any
    /// dispatch path — same mechanism `gpui_component`'s own always-on
    /// `ToggleInspector` shortcut uses. Since opening a dialog never reads/
    /// writes any `JadeApp` field anyway (same reasoning `open_settings_
    /// dialog` below is already a plain function, not a method), there's no
    /// `Entity<Self>` to round-trip back through here either.
    pub(super) fn open_command_palette(window: &mut Window, cx: &mut App) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            // `Command` never focuses itself on mount — without this,
            // Enter/arrow-key navigation has no focused element inside the
            // dialog to dispatch to at all (same reasoning the global
            // Ctrl+P binding itself needs *some* focused leaf to bubble up
            // from).
            state.update(cx, |state, cx| state.focus(window, cx));
            dialog.title("Command Palette").child(
                Command::new(&state)
                    // Hosted inside a `Dialog`, which already draws its own
                    // frame — `Command`'s own border would double up.
                    .bordered(false)
                    .placeholder("Type a command...")
                    .item(CommandItem::new().label("Open Settings"))
                    .on_confirm(|_index_path, window, cx| {
                        window.close_dialog(cx);
                        JadeApp::open_settings_dialog(window, cx);
                    })
                    .debug_name("command-palette"),
            )
        });
    }

    /// The Settings dialog; for now only the theme dropdown (text size and
    /// editor preferences are to come). A plain associated function: it
    /// never needs `JadeApp`'s state. The theme list is read once on open.
    pub(super) fn open_settings_dialog(window: &mut Window, cx: &mut App) {
        let options: Vec<ThemeOption> = ThemeEngine::available(cx)
            .into_iter()
            .map(|choice| ThemeOption {
                id: choice.id,
                name: choice.name.into(),
            })
            .collect();
        let active = ThemeEngine::global(cx).active_id().to_owned();
        let selected = options.iter().position(|o| o.id == active);
        let select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(options),
                selected.map(IndexPath::new),
                window,
                cx,
            )
        });
        cx.subscribe(
            &select,
            |_, event: &SelectEvent<SearchableVec<ThemeOption>>, cx| {
                let SelectEvent::Confirm(Some(id)) = event else {
                    return;
                };
                if let Err(messages) = ThemeEngine::choose(id, cx) {
                    for message in messages {
                        eprintln!("jade: {message}");
                    }
                }
            },
        )
        .detach();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let muted = ThemeEngine::color(cx, Role::TextMuted);
            dialog.title("Settings").child(
                div()
                    .id("settings-panel")
                    .test_support()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_color(muted).child("Theme"))
                    .child(
                        div()
                            .child(Select::new(&select).id("theme-select"))
                            .debug_name("theme-select"),
                    )
                    .child(div().text_color(muted).child("Text"))
                    .child(
                        div()
                            .child(
                                Switch::new("justify-switch")
                                    .label("Justify text")
                                    .checked(TextSettings::justify(cx))
                                    .on_click(|on, _window, cx| {
                                        if let Err(message) = TextSettings::choose_justify(*on, cx)
                                        {
                                            eprintln!("jade: {message}");
                                        }
                                    }),
                            )
                            .debug_name("justify-switch"),
                    )
                    .debug_name("settings-panel"),
            )
        });
    }

    pub(super) fn open_rename_dialog(
        id: String,
        current_title: String,
        app: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current_title));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let app = app.clone();
            let id = id.clone();
            dialog
                .title("Rename note")
                .child(
                    Input::new(&input)
                        .id("rename-input")
                        .debug_name("rename-input"),
                )
                // `on_ok` is what the dialog's built-in Enter-to-confirm keybinding
                // dispatches to, regardless of which footer buttons are rendered —
                // without it, Enter silently does nothing for a custom footer.
                .on_ok({
                    let input = input.clone();
                    let app = app.clone();
                    let id = id.clone();
                    move |_, window, cx| {
                        Self::commit_rename(&id, &input, &app, window, cx);
                        true
                    }
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel-rename")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx))
                                .debug_name("cancel-rename"),
                        )
                        .child(
                            Button::new("confirm-rename")
                                .label("Rename")
                                .on_click(move |_, window, cx| {
                                    Self::commit_rename(&id, &input, &app, window, cx);
                                    window.close_dialog(cx);
                                })
                                .debug_name("confirm-rename"),
                        ),
                )
        });
    }

    pub(super) fn open_delete_dialog(
        id: String,
        title: String,
        app: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let app = app.clone();
            let id = id.clone();
            alert
                .confirm()
                .title("Delete note")
                .description(format!("Delete \"{title}\"? This can't be undone."))
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _window, cx| {
                    app.update(cx, |app, cx| {
                        app.delete_note(&id, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn open_rename_folder_dialog(
        id: String,
        current_name: String,
        app: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current_name));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let app = app.clone();
            let id = id.clone();
            dialog
                .title("Rename folder")
                .child(
                    Input::new(&input)
                        .id("rename-folder-input")
                        .debug_name("rename-folder-input"),
                )
                // See the comment on the note rename dialog's `on_ok` for why this is
                // needed in addition to the footer button: it's what Enter dispatches to.
                .on_ok({
                    let input = input.clone();
                    let app = app.clone();
                    let id = id.clone();
                    move |_, window, cx| {
                        Self::commit_rename_folder(&id, &input, &app, window, cx);
                        true
                    }
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel-rename-folder")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx))
                                .debug_name("cancel-rename-folder"),
                        )
                        .child(
                            Button::new("confirm-rename-folder")
                                .label("Rename")
                                .on_click(move |_, window, cx| {
                                    Self::commit_rename_folder(&id, &input, &app, window, cx);
                                    window.close_dialog(cx);
                                })
                                .debug_name("confirm-rename-folder"),
                        ),
                )
        });
    }

    pub(super) fn open_delete_folder_dialog(
        id: String,
        name: String,
        app: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let app = app.clone();
            let id = id.clone();
            alert
                .confirm()
                .title("Delete folder")
                .description(format!(
                    "Delete \"{name}\" and everything inside it? This can't be undone."
                ))
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _window, cx| {
                    app.update(cx, |app, cx| {
                        app.delete_folder(&id, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn open_create_folder_dialog(
        parent: Option<String>,
        app: Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder name"));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input = input.clone();
            let app = app.clone();
            let parent = parent.clone();
            dialog
                .title("New folder")
                .child(
                    Input::new(&input)
                        .id("create-folder-input")
                        .debug_name("create-folder-input"),
                )
                // See the comment on the rename dialog's `on_ok` for why this is
                // needed in addition to the footer button: it's what Enter dispatches to.
                .on_ok({
                    let input = input.clone();
                    let app = app.clone();
                    let parent = parent.clone();
                    move |_, _window, cx| {
                        Self::commit_create_folder(&parent, &input, &app, cx);
                        true
                    }
                })
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("cancel-create-folder")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx))
                                .debug_name("cancel-create-folder"),
                        )
                        .child(
                            Button::new("confirm-create-folder")
                                .label("Create")
                                .on_click(move |_, window, cx| {
                                    Self::commit_create_folder(&parent, &input, &app, cx);
                                    window.close_dialog(cx);
                                })
                                .debug_name("confirm-create-folder"),
                        ),
                )
        });
    }
}

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

/// The heading sizes the Settings dialog offers (`heading.scale`, H1..H6):
/// `[1; 6]` is the plain look, the rest are heading sizes to start from.
const HEADING_SIZES: [(&str, [f32; 6]); 4] = [
    ("Plain (same as text)", [1.0; 6]),
    ("Subtle", [1.5, 1.3, 1.15, 1.05, 1.0, 1.0]),
    ("Normal", [2.0, 1.6, 1.35, 1.15, 1.0, 0.9]),
    ("Large", [2.4, 1.9, 1.6, 1.3, 1.1, 1.0]),
];

/// One entry of the Settings heading-size dropdown; the value is its index in
/// [`HEADING_SIZES`].
#[derive(Clone)]
struct HeadingOption {
    index: usize,
}

impl gpui_kit::component::select::SelectItem for HeadingOption {
    type Value = usize;

    fn title(&self) -> SharedString {
        HEADING_SIZES[self.index].0.into()
    }

    fn value(&self) -> &usize {
        &self.index
    }
}

/// A slider of the Settings dialog: shows `value` live as it moves and
/// remembers it in `[section] key` of `config.toml` when let go.
struct SettingSlider {
    state: Entity<gpui_kit::component::slider::SliderState>,
}

impl SettingSlider {
    /// Wires a slider over `range`: `apply` takes its value now (on every move),
    /// and `(section, key)` is written when it is released.
    fn new(
        (min, max, value): (f32, f32, f32),
        (section, key): (&'static str, &'static str),
        apply: fn(&mut crate::settings::Typography, f32),
        cx: &mut App,
    ) -> Self {
        use gpui_kit::component::slider::{SliderEvent, SliderState};
        let state = cx.new(|_| {
            SliderState::new()
                .min(min)
                .max(max)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&state, move |_, event: &SliderEvent, cx| match event {
            SliderEvent::Change(v) => {
                TextSettings::apply_typography(|t| apply(t, v.start()), cx);
            }
            SliderEvent::Release(v) => {
                let text = format!("{}", v.start());
                if let Err(message) = TextSettings::remember(section, key, &text, cx) {
                    eprintln!("jade: {message}");
                }
            }
        })
        .detach();
        Self { state }
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

    /// The heading-size dropdown, showing the preset equal to `scale` (none if
    /// it is custom); a pick applies and is remembered.
    fn heading_select(
        scale: [f32; 6],
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SelectState<SearchableVec<HeadingOption>>> {
        let headings = cx.new(|cx| {
            let current = HEADING_SIZES
                .iter()
                .position(|(_, preset)| *preset == scale);
            SelectState::new(
                SearchableVec::new(
                    (0..HEADING_SIZES.len())
                        .map(|index| HeadingOption { index })
                        .collect::<Vec<_>>(),
                ),
                current.map(IndexPath::new),
                window,
                cx,
            )
        });
        cx.subscribe(
            &headings,
            |_, event: &SelectEvent<SearchableVec<HeadingOption>>, cx| {
                let SelectEvent::Confirm(Some(index)) = event else {
                    return;
                };
                let scale = HEADING_SIZES[*index].1;
                TextSettings::apply_typography(|t| t.heading.scale = scale, cx);
                let list: Vec<String> = scale.iter().map(|v| v.to_string()).collect();
                let text = format!("[{}]", list.join(", "));
                if let Err(message) = TextSettings::remember("heading", "scale", &text, cx) {
                    eprintln!("jade: {message}");
                }
            },
        )
        .detach();
        headings
    }

    /// The theme dropdown, showing the active theme; a pick switches it.
    fn theme_select(
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SelectState<SearchableVec<ThemeOption>>> {
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
        select
    }

    /// The text size and corner radius sliders, at the current values.
    fn setting_sliders(
        typography: &crate::settings::Typography,
        cx: &mut App,
    ) -> (SettingSlider, SettingSlider) {
        let size = SettingSlider::new(
            (8.0, 40.0, typography.font.size),
            ("font", "size"),
            |t, v| t.font.size = v,
            cx,
        );
        let radius = SettingSlider::new(
            (0.0, 16.0, typography.shape.radius.px(32.0)),
            ("shape", "radius"),
            |t, v| t.shape.radius = crate::settings::Radius::Px(v),
            cx,
        );
        (size, radius)
    }

    /// The Settings dialog: theme, text size, heading size, corner radius and
    /// justify. A plain associated function: it never needs `JadeApp`'s state.
    /// The theme list is read once on open.
    pub(super) fn open_settings_dialog(window: &mut Window, cx: &mut App) {
        let select = Self::theme_select(window, cx);
        let typography = TextSettings::typography(cx);
        let (size, radius) = Self::setting_sliders(&typography, cx);
        let headings = Self::heading_select(typography.heading.scale, window, cx);
        window.open_dialog(cx, move |dialog, _window, cx| {
            let muted = ThemeEngine::color(cx, Role::TextMuted);
            let shown = TextSettings::typography(cx);
            let labelled = |text: String, id: &'static str, slider: &SettingSlider| {
                div().child(div().text_color(muted).child(text)).child(
                    div()
                        .child(gpui_kit::component::slider::Slider::new(&slider.state))
                        .debug_name(id),
                )
            };
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
                    .child(labelled(
                        format!("Text size: {} px", shown.font.size),
                        "font-size-slider",
                        &size,
                    ))
                    .child(div().text_color(muted).child("Heading size"))
                    .child(
                        div()
                            .child(Select::new(&headings).id("heading-size-select"))
                            .debug_name("heading-size-select"),
                    )
                    .child(labelled(
                        format!("Corner radius: {} px", shown.shape.radius.px(32.0)),
                        "corner-radius-slider",
                        &radius,
                    ))
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

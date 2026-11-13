use std::path::PathBuf;

mod text_editor;

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Theme, WindowExt};
use gpui_kit::*;
use jade_core::Vault;
use text_editor::{Editor, EditorEvent, InsertTab};

/// IBM Plex — the typeface Zed itself uses — bundled under the SIL Open Font
/// License (OFL 1.1, see `assets/fonts/*/LICENSE.txt`), which explicitly
/// permits embedding/redistributing it with an application.
const IBM_PLEX_SANS: &str = "IBM Plex Sans";
const IBM_PLEX_MONO: &str = "IBM Plex Mono";

fn install_fonts(cx: &mut App) {
    use std::borrow::Cow;

    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Italic.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Bold.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-BoldItalic.ttf")
                    .as_slice(),
            ),
        ])
        .expect("failed to load bundled IBM Plex fonts");

    let theme = Theme::global_mut(cx);
    theme.font_family = IBM_PLEX_SANS.into();
    theme.mono_font_family = IBM_PLEX_MONO.into();
}

actions!(jade, [RenameNote, DeleteNote, OpenCommandPalette]);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SidebarTab {
    #[default]
    Vault,
    Search,
}

struct JadeApp {
    vault: Vault,
    selected_id: Option<String>,
    /// The currently open note's editor, if a note is selected — Jade's own
    /// custom-built `text_editor::Editor` (see `CLAUDE.md`'s "Custom editor"
    /// section), not `gpui-component`'s `Textarea`. One continuous editor for
    /// the whole note; Live Preview (Phase 2) will change how it renders
    /// per-block internally rather than swapping between separate widgets.
    editor: Option<Entity<Editor>>,
    _editor_subscription: Option<Subscription>,
    error: Option<String>,
    sidebar_focus: FocusHandle,
    collapsed_folders: std::collections::HashSet<String>,
    sidebar_tab: SidebarTab,
    search_input: Entity<InputState>,
    search_query: String,
    _search_subscription: Subscription,
}

/// The direct parent folder id of a note/folder id, or `None` at the vault root.
fn parent_of(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(parent, _)| parent)
}

impl JadeApp {
    fn new(vault_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar_focus = cx.focus_handle();

        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search notes..."));
        let search_subscription = cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                cx.notify();
            }
        });

        let (vault, error) = match Vault::open(&vault_path) {
            Ok(vault) => (vault, None),
            Err(err) => (Vault::empty(vault_path), Some(err.to_string())),
        };

        Self {
            vault,
            selected_id: None,
            editor: None,
            _editor_subscription: None,
            error,
            sidebar_focus,
            collapsed_folders: Default::default(),
            sidebar_tab: SidebarTab::default(),
            search_input,
            search_query: String::new(),
            _search_subscription: search_subscription,
        }
    }

    fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // Flush the outgoing note ourselves before swapping `self.editor` out: an
        // `EditorEvent::Changed` from the last keystroke could still be queued, and
        // without this, switching notes right after typing would drop it.
        self.flush_editor(cx);

        let Some(note) = self.vault.note(&id) else {
            return;
        };
        let text = note.content.clone();
        let editor = cx.new(|cx| Editor::new(text, cx));
        let subscription = cx.subscribe(&editor, |this, _editor, event, cx| {
            if matches!(event, EditorEvent::Changed) {
                this.autosave_editor(cx);
            }
        });
        editor.read(cx).focus_handle(cx).focus(window, cx);

        self.selected_id = Some(id);
        self.editor = Some(editor);
        self._editor_subscription = Some(subscription);
        cx.notify();
    }

    /// Writes the open editor's (if any) current text back into the vault and
    /// drops it. Call before anything that would make the editor's content
    /// stale to keep editing against: switching notes, or renaming the open
    /// note's folder.
    fn flush_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(note_id)) = (self.editor.take(), self.selected_id.clone()) else {
            return;
        };
        self._editor_subscription = None;
        let text = editor.read(cx).buffer().text().to_string();
        let _ = self.vault.update_note_content(&note_id, text);
    }

    /// Persists the open editor's current text to the vault on every
    /// `EditorEvent::Changed`, and re-renders so e.g. the backlinks panel
    /// reflects newly-typed wikilinks live, not just on the next note switch.
    fn autosave_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(note_id)) = (self.editor.clone(), self.selected_id.clone())
        else {
            return;
        };
        let text = editor.read(cx).buffer().text().to_string();
        let _ = self.vault.update_note_content(&note_id, text);
        cx.notify();
    }

    fn create_note(&mut self, parent: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(id) = self.vault.create_note("Untitled", parent) {
            self.open_note(id, window, cx);
            self.sidebar_focus.focus(window, cx);
        }
    }

    fn create_folder(&mut self, name: &str, parent: Option<String>, cx: &mut Context<Self>) {
        let _ = self.vault.create_folder(name, parent.as_deref());
        cx.notify();
    }

    fn toggle_folder(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(folder_id) {
            self.collapsed_folders.insert(folder_id.to_string());
        }
        cx.notify();
    }

    fn rename_note(&mut self, id: &str, new_title: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Flush first: renaming moves the note to a new id, so a flush keyed on
        // its *old* id (which the editor's edits are still relative to) has to
        // happen before that, not after.
        self.flush_editor(cx);

        let Ok(new_id) = self.vault.rename_note(id, new_title) else {
            return;
        };
        if self.selected_id.as_deref() == Some(id) {
            self.open_note(new_id, window, cx);
        } else {
            cx.notify();
        }
    }

    fn delete_note(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.vault.delete_note(id).is_ok() && self.selected_id.as_deref() == Some(id) {
            self.selected_id = None;
            self.editor = None;
            self._editor_subscription = None;
        }
        cx.notify();
    }

    fn rename_folder(&mut self, id: &str, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Flush the open note first (same reasoning as `open_note`'s own flush):
        // renaming the folder moves it to a new id, so a flush keyed on its *old*
        // id must happen before that, not after.
        self.flush_editor(cx);

        let Ok(new_id) = self.vault.rename_folder(id, new_name) else {
            return;
        };

        if self.collapsed_folders.remove(id) {
            self.collapsed_folders.insert(new_id.clone());
        }

        let prefix = format!("{id}/");
        let moved_selection = self
            .selected_id
            .clone()
            .and_then(|selected| selected.strip_prefix(&prefix).map(str::to_string));
        match moved_selection {
            Some(rest) => self.open_note(format!("{new_id}/{rest}"), window, cx),
            None => cx.notify(),
        }
    }

    fn delete_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.vault.delete_folder(id).is_ok() {
            let prefix = format!("{id}/");
            self.collapsed_folders.retain(|f| f != id && !f.starts_with(&prefix));
            if self.selected_id.as_deref().is_some_and(|sel| sel.starts_with(&prefix)) {
                self.selected_id = None;
                self.editor = None;
                self._editor_subscription = None;
            }
        }
        cx.notify();
    }

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
    fn open_command_palette(window: &mut Window, cx: &mut App) {
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
                    }),
            )
        });
    }

    /// A placeholder — nothing here is wired up to anything yet (themes,
    /// text size, general editor preferences are all still to come, see
    /// CLAUDE.md's Backlog). A plain associated function, not a `&mut self`
    /// method like the dialog-openers below it: unlike those, it never
    /// needs to call back into `JadeApp`'s own state, so there's no reason
    /// to thread an `Entity<Self>` through it.
    fn open_settings_dialog(window: &mut Window, cx: &mut App) {
        window.open_dialog(cx, |dialog, _window, _cx| {
            dialog.title("Settings").child(
                div()
                    .id("settings-panel")
                    .test_support()
                    .p_4()
                    .text_color(rgb(0x888888))
                    .child("Settings aren't implemented yet — themes, text size, and general editor preferences are coming soon."),
            )
        });
    }

    fn rename_selected(&mut self, _: &RenameNote, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_id.clone() {
            let title = self.vault.note(&id).map(|n| n.title.clone()).unwrap_or_default();
            Self::open_rename_dialog(id, title, cx.entity(), window, cx);
        }
    }

    fn delete_selected(&mut self, _: &DeleteNote, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_id.clone() {
            let title = self.vault.note(&id).map(|n| n.title.clone()).unwrap_or_default();
            Self::open_delete_dialog(id, title, cx.entity(), window, cx);
        }
    }

    fn commit_rename(id: &str, input: &Entity<InputState>, app: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let new_title = input.read(cx).value().to_string();
        app.update(cx, |app, cx| {
            app.rename_note(id, &new_title, window, cx);
        });
    }

    fn open_rename_dialog(
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
                .child(Input::new(&input).id("rename-input"))
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
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("confirm-rename").label("Rename").on_click(
                            move |_, window, cx| {
                                Self::commit_rename(&id, &input, &app, window, cx);
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    fn open_delete_dialog(
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

    fn commit_rename_folder(id: &str, input: &Entity<InputState>, app: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let new_name = input.read(cx).value().to_string();
        app.update(cx, |app, cx| {
            app.rename_folder(id, &new_name, window, cx);
        });
    }

    fn open_rename_folder_dialog(
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
                .child(Input::new(&input).id("rename-folder-input"))
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
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("confirm-rename-folder").label("Rename").on_click(
                            move |_, window, cx| {
                                Self::commit_rename_folder(&id, &input, &app, window, cx);
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    fn open_delete_folder_dialog(
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

    fn commit_create_folder(
        parent: &Option<String>,
        input: &Entity<InputState>,
        app: &Entity<Self>,
        cx: &mut App,
    ) {
        let name = input.read(cx).value().to_string();
        if !name.trim().is_empty() {
            app.update(cx, |app, cx| {
                app.create_folder(&name, parent.clone(), cx);
            });
        }
    }

    fn open_create_folder_dialog(
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
                .child(Input::new(&input).id("create-folder-input"))
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
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("confirm-create-folder").label("Create").on_click(
                            move |_, window, cx| {
                                Self::commit_create_folder(&parent, &input, &app, cx);
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    /// Renders the notes and (non-collapsed) subfolders directly inside `parent`
    /// (vault root if `None`), indented by `depth`, recursing into subfolders.
    fn render_tree_level(
        &self,
        parent: Option<&str>,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut folder_ids: Vec<&String> =
            self.vault.folders.iter().filter(|id| parent_of(id) == parent).collect();
        folder_ids.sort();

        let mut note_entries: Vec<(&String, &str)> = self
            .vault
            .notes
            .iter()
            .filter(|(id, _)| parent_of(id) == parent)
            .map(|(id, note)| (id, note.title.as_str()))
            .collect();
        note_entries.sort_by_key(|(_, title)| *title);

        let mut rows = Vec::new();
        for folder_id in folder_ids {
            rows.push(self.render_folder_row(folder_id, depth, cx));
            if !self.collapsed_folders.contains(folder_id.as_str()) {
                rows.extend(self.render_tree_level(Some(folder_id), depth + 1, cx));
            }
        }
        for (id, title) in note_entries {
            rows.push(self.render_note_row(id, title, depth, cx));
        }
        rows
    }

    fn render_folder_row(&self, folder_id: &str, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let collapsed = self.collapsed_folders.contains(folder_id);
        let name = folder_id.rsplit('/').next().unwrap_or(folder_id);
        let disclosure = if collapsed { "\u{25b8}" } else { "\u{25be}" };
        let toggle_id = folder_id.to_string();
        let menu_parent = folder_id.to_string();
        let menu_name = name.to_string();
        let app_entity = cx.entity();

        div()
            .id(format!("folder-{folder_id}"))
            .test_support()
            .ml(px(depth as f32 * 14.))
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .child(format!("{disclosure} {name}"))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_folder(&toggle_id, cx);
            }))
            .context_menu(move |menu, _window, _cx| {
                let note_entity = app_entity.clone();
                let note_parent = menu_parent.clone();
                let folder_entity = app_entity.clone();
                let folder_parent = menu_parent.clone();
                let rename_entity = app_entity.clone();
                let rename_id = menu_parent.clone();
                let rename_name = menu_name.clone();
                let delete_entity = app_entity.clone();
                let delete_id = menu_parent.clone();
                let delete_name = menu_name.clone();
                menu.item(PopupMenuItem::new("New note").on_click(move |_, window, cx| {
                    note_entity.update(cx, |app, cx| {
                        app.create_note(Some(&note_parent), window, cx);
                    });
                }))
                .item(PopupMenuItem::new("New folder").on_click(move |_, window, cx| {
                    JadeApp::open_create_folder_dialog(
                        Some(folder_parent.clone()),
                        folder_entity.clone(),
                        window,
                        cx,
                    );
                }))
                .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                    JadeApp::open_rename_folder_dialog(
                        rename_id.clone(),
                        rename_name.clone(),
                        rename_entity.clone(),
                        window,
                        cx,
                    );
                }))
                .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                    JadeApp::open_delete_folder_dialog(
                        delete_id.clone(),
                        delete_name.clone(),
                        delete_entity.clone(),
                        window,
                        cx,
                    );
                }))
            })
            .into_any_element()
    }

    fn render_note_row(
        &self,
        id: &str,
        title: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected_id.as_deref() == Some(id);
        let item_id = id.to_string();
        let menu_id = id.to_string();
        let menu_title = title.to_string();
        let app_entity = cx.entity();

        let mut item = div()
            .id(id.to_string())
            .test_support()
            .ml(px(depth as f32 * 14.))
            .px_2()
            .py_1()
            .rounded_md()
            .child(title.to_string());
        if is_selected {
            item = item.bg(rgb(0xd0d0f0));
        }
        item = item.on_click(cx.listener(move |this, _event, window, cx| {
            this.open_note(item_id.clone(), window, cx);
            this.sidebar_focus.focus(window, cx);
        }));

        item.context_menu(move |menu, _window, _cx| {
            let rename_entity = app_entity.clone();
            let rename_id = menu_id.clone();
            let rename_title = menu_title.clone();
            let delete_entity = app_entity.clone();
            let delete_id = menu_id.clone();
            let delete_title = menu_title.clone();
            menu.item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                JadeApp::open_rename_dialog(
                    rename_id.clone(),
                    rename_title.clone(),
                    rename_entity.clone(),
                    window,
                    cx,
                );
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                JadeApp::open_delete_dialog(
                    delete_id.clone(),
                    delete_title.clone(),
                    delete_entity.clone(),
                    window,
                    cx,
                );
            }))
        })
        .into_any_element()
    }

    fn render_sidebar_tab_button(
        &self,
        tab: SidebarTab,
        label: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_active = self.sidebar_tab == tab;
        let id = match tab {
            SidebarTab::Vault => "sidebar-tab-vault",
            SidebarTab::Search => "sidebar-tab-search",
        };

        let mut item = div()
            .id(id)
            .test_support()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .child(label.to_string());
        if is_active {
            item = item.bg(rgb(0xd0d0f0)).font_weight(FontWeight::BOLD);
        }
        item.on_click(cx.listener(move |this, _event, _window, cx| {
            this.sidebar_tab = tab;
            cx.notify();
        }))
        .into_any_element()
    }

    /// The "Search" sidebar tab: a live-filtered list of notes matching the
    /// query, by title or content substring (via `Vault::search`).
    fn render_search_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(Input::new(&self.search_input).id("search-input"));

        let query = self.search_query.trim();
        if query.is_empty() {
            panel = panel.child(
                div()
                    .text_color(rgb(0x888888))
                    .child("Type to search notes by title or content."),
            );
        } else {
            let mut results: Vec<(String, String)> = self
                .vault
                .search(query)
                .into_iter()
                .map(|note| (note.id.clone(), note.title.clone()))
                .collect();
            results.sort_by(|a, b| a.1.cmp(&b.1));

            panel = panel.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(format!("Results ({})", results.len())),
            );
            if results.is_empty() {
                panel = panel
                    .child(div().text_color(rgb(0x888888)).child("No matching notes."));
            } else {
                for (id, title) in results {
                    let target_id = id.clone();
                    panel = panel.child(
                        div()
                            .id(format!("search-result-{id}"))
                            .test_support()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0xe8e8f8)))
                            .child(title)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.open_note(target_id.clone(), window, cx);
                            })),
                    );
                }
            }
        }
        panel.into_any_element()
    }

}

impl Render for JadeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_bar = div()
            .flex()
            .flex_row()
            .gap_1()
            .mb_2()
            .child(self.render_sidebar_tab_button(SidebarTab::Vault, "Vault", cx))
            .child(self.render_sidebar_tab_button(SidebarTab::Search, "Search", cx));

        // The "NoteList" key context (and its F2/Backspace bindings) is scoped to
        // just the vault tree, not the whole sidebar — otherwise Backspace while
        // typing in the search input above would delete the selected note instead
        // of erasing a character.
        let sidebar_content: AnyElement = match self.sidebar_tab {
            SidebarTab::Vault => {
                let tree_rows = self.render_tree_level(None, 0, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .key_context("NoteList")
                    .track_focus(&self.sidebar_focus)
                    .on_action(cx.listener(Self::rename_selected))
                    .on_action(cx.listener(Self::delete_selected))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                Button::new("new-note")
                                    .label("New note")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.create_note(None, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("new-folder")
                                    .label("New folder")
                                    .on_click(cx.listener(|_this, _event, window, cx| {
                                        let app_entity = cx.entity();
                                        JadeApp::open_create_folder_dialog(
                                            None, app_entity, window, cx,
                                        );
                                    })),
                            ),
                    )
                    .children(tree_rows)
                    .into_any_element()
            }
            SidebarTab::Search => self.render_search_panel(cx),
        };

        let sidebar = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(220.))
            .p_2()
            .border_r_1()
            .child(tab_bar)
            .child(sidebar_content);

        let content: AnyElement = if let Some(err) = &self.error {
            div().p_4().child(format!("Could not open vault: {err}")).into_any_element()
        } else if let Some(note_id) = self.selected_id.clone() {
            let mut backlink_entries: Vec<(String, String)> = self
                .vault
                .backlinks()
                .get(&note_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|source_id| {
                    self.vault.note(&source_id).map(|n| (source_id, n.title.clone()))
                })
                .collect();
            backlink_entries.sort_by(|a, b| a.1.cmp(&b.1));

            let mut backlinks_panel = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_t_1()
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(format!("Backlinks ({})", backlink_entries.len())),
                );

            if backlink_entries.is_empty() {
                backlinks_panel = backlinks_panel
                    .child(div().text_color(rgb(0x888888)).child("No backlinks yet."));
            } else {
                for (source_id, title) in backlink_entries {
                    let target_id = source_id.clone();
                    backlinks_panel = backlinks_panel.child(
                        div()
                            .id(format!("backlink-{source_id}"))
                            .test_support()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0xe8e8f8)))
                            .child(title)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.open_note(target_id.clone(), window, cx);
                            })),
                    );
                }
            }

            let mut editor_pane = div()
                .id("note-editor-pane")
                .flex_1()
                .overflow_y_scroll()
                .children(self.editor.clone());
            // Tracked so `text_editor::element`'s `autoscroll_to_cursor` can
            // scroll this container itself (see `Editor::scroll_handle`'s
            // doc comment) — without this, moving the cursor below the
            // visible area via the keyboard has nothing to adjust.
            if let Some(editor) = &self.editor {
                editor_pane = editor_pane.track_scroll(editor.read(cx).scroll_handle());
            }

            div()
                .flex()
                .flex_col()
                .size_full()
                .p_4()
                .child(editor_pane)
                .child(backlinks_panel)
                .into_any_element()
        } else {
            div()
                .p_4()
                .child("Select a note, or create a new one.")
                .into_any_element()
        };

        div()
            .flex()
            .flex_row()
            .size_full()
            .child(sidebar)
            .child(div().flex_1().child(content))
    }
}

fn main() {
    let vault_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("vault"));

    // Registers the bundled Lucide icon SVGs (`Dialog`'s own close "x",
    // `Button::icon(...)`, etc.) with GPUI's asset loader — without this,
    // every `IconName` renders as empty (the button itself still paints,
    // just with nothing inside it), not just the close button specifically.
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        install_fonts(cx);
        cx.bind_keys([
            KeyBinding::new("f2", RenameNote, Some("NoteList")),
            KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
            KeyBinding::new("tab", InsertTab, Some("TextEditor")),
            KeyBinding::new("ctrl-p", OpenCommandPalette, None),
        ]);
        bind_open_command_palette(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
            cx.new(|cx| JadeApp::new(vault_path.clone(), window, cx))
        })
        .expect("failed to open window");
    });
}

/// Registers `OpenCommandPalette`'s handler globally (`cx.on_action`, not an
/// element-attached `.on_action(cx.listener(...))`) — see `JadeApp::
/// open_command_palette`'s own doc comment for why this specific mechanism
/// is required for Ctrl+P to work with nothing in the app currently
/// focused. Split out so both `main()` and the test helper `open_app` call
/// the exact same registration, same reasoning `cx.bind_keys([...])` is
/// already duplicated between the two rather than risking them drifting
/// apart.
fn bind_open_command_palette(cx: &mut App) {
    cx.on_action(|_: &OpenCommandPalette, cx| {
        // Not `cx.active_window()` — that's a genuine OS-level "which
        // window has platform focus" query (`App::active_window` delegates
        // straight to `self.platform.active_window()`), which is legitimately
        // always `None` under the headless test platform (no real window
        // manager to ask), and not something to lean on for a single-window
        // app regardless. `cx.windows()` just lists every window GPUI
        // itself has open, with no OS-focus dependency at all.
        let Some(window_handle) = cx.windows().into_iter().next() else { return };
        cx.defer(move |cx| {
            _ = window_handle.update(cx, |_, window, cx| {
                JadeApp::open_command_palette(window, cx);
            });
        });
    });
}

// Deliberately not `use super::*;` / `use gpui_kit::*;` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows the plain `#[test]` its own expansion relies on and recurses forever.
#[cfg(test)]
mod tests {
    use super::{DeleteNote, Editor, InsertTab, JadeApp, OpenCommandPalette, RenameNote};
    use gpui_kit::component::WindowExt;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, Focusable, KeyBinding, TestAppContext, WindowOptions, point, px,
    };
    use std::path::PathBuf;
    use tempfile::{TempDir, tempdir};

    fn temp_vault() -> TempDir {
        tempdir().unwrap()
    }

    fn open_app(cx: &mut TestAppContext, vault_path: PathBuf) -> (AnyWindowHandle, Entity<JadeApp>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("f2", RenameNote, Some("NoteList")),
                KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
                KeyBinding::new("tab", InsertTab, Some("TextEditor")),
                KeyBinding::new("ctrl-p", OpenCommandPalette, None),
            ]);
            super::bind_open_command_palette(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                cx.new(|cx| JadeApp::new(vault_path.clone(), window, cx))
            })
            .expect("open test window")
        })
    }

    #[gpui_kit::test]
    fn clicking_sidebar_note_opens_it(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "No outgoing links.").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note B", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note B".to_string()));
    }

    #[gpui_kit::test]
    fn clicking_backlink_navigates_to_source_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "No outgoing links.").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note B", cx);
            window.render_frame(cx);
            window.click("backlink-Note A", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note A".to_string()));
    }

    #[gpui_kit::test]
    fn moving_the_cursor_below_the_visible_area_scrolls_it_into_view(cx: &mut TestAppContext) {
        let dir = temp_vault();
        let lines: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.path().join("Long Note.md"), lines.join("\n")).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long Note", cx);
        })
        .unwrap();

        let editor = app.read_with(cx, |app, _| app.editor.clone()).expect("opening a note should create an editor");

        let offset_before = editor.read_with(cx, |editor, _| editor.scroll_handle().offset());
        assert_eq!(offset_before.y, px(0.), "a freshly opened note should start scrolled to the top");

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Near the top-left corner — definitely within the initial,
            // unscrolled viewport, unlike a plain `click` on this 300-line
            // element, whose default (centered) target position could land
            // well below the visible area.
            window.click_at("editor", point(px(1.), px(1.)), cx);
            editor.update(cx, |editor, cx| {
                editor.buffer_mut().move_to_end(false);
                cx.notify();
            });
            window.render_frame(cx);
        })
        .unwrap();

        let offset_after = editor.read_with(cx, |editor, _| editor.scroll_handle().offset());
        assert!(
            offset_after.y < px(0.),
            "moving the cursor to the end of a long note should scroll it into view, got {offset_after:?}"
        );
    }

    #[gpui_kit::test]
    fn a_line_too_wide_for_the_pane_wraps_instead_of_overflowing(cx: &mut TestAppContext) {
        // Unlike `text_editor::element`'s own tests (a bare `Editor` with no
        // surrounding container), only a *real* `note-editor-pane` div
        // (`.track_scroll(editor.scroll_handle())`, see `main.rs`'s own
        // rendering code) gives `Editor::last_autoscrolled_cursor`'s sibling
        // concern — `render_editor`'s pre-layout `measure_document` call —
        // an actual tracked width to measure wrapping against; a bare
        // `Editor` always falls back to "assume no wrap" (`Pixels::MAX`),
        // so this needs the full app to verify wrapping-aware sizing at all.
        let dir = temp_vault();
        let long: &'static str = Box::leak("word ".repeat(500).into_boxed_str());
        std::fs::write(dir.path().join("Long Line.md"), long).unwrap();
        std::fs::write(dir.path().join("Short.md"), "short").unwrap();

        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Short", cx);
        })
        .unwrap();
        let short_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long Line", cx);
        })
        .unwrap();
        let long_height = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("editor").bounds().size.height
            })
            .unwrap();

        assert!(
            long_height > short_height,
            "a line too wide for the note-editor-pane should wrap onto additional rows \
             (short={short_height:?}, long={long_height:?})"
        );
    }

    #[gpui_kit::test]
    fn clicking_a_wrapped_lines_second_row_in_the_real_app_lands_further_into_the_text(cx: &mut TestAppContext) {
        let dir = temp_vault();
        let long: &'static str = Box::leak("a".repeat(2000).into_boxed_str());
        std::fs::write(dir.path().join("Long Line.md"), long).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long Line", cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone()).expect("opening a note should create an editor");

        let line_height = cx.update_window(window, |_, window, _cx| window.line_height()).unwrap();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at("editor", point(px(1.), px(1.)), cx);
        })
        .unwrap();
        let first_row_cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at("editor", point(px(1.), line_height + px(1.)), cx);
        })
        .unwrap();
        let second_row_cursor = editor.read_with(cx, |editor, _| editor.buffer().cursor());

        assert!(
            second_row_cursor > first_row_cursor,
            "clicking the wrapped line's second visual row should land further into the text than its first row \
             (first={first_row_cursor}, second={second_row_cursor})"
        );
    }

    #[gpui_kit::test]
    fn scrolling_away_from_the_cursor_does_not_get_snapped_back(cx: &mut TestAppContext) {
        // Regression test: `render_editor`'s whole element tree (including
        // its `canvas` paint closure) is rebuilt on *every* actual screen
        // repaint, not just ones where the cursor moved — a plain
        // mouse-wheel scroll alone triggers one too. Before
        // `Editor::last_autoscrolled_cursor` existed, that meant
        // `autoscroll_to_cursor` ran unconditionally on every such repaint
        // and snapped the view straight back to the (unchanged) cursor's
        // line the moment it scrolled far enough to leave the viewport —
        // making it impossible to scroll away from the cursor at all.
        let dir = temp_vault();
        let lines: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        std::fs::write(dir.path().join("Long Note.md"), lines.join("\n")).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long Note", cx);
        })
        .unwrap();

        let editor = app.read_with(cx, |app, _| app.editor.clone()).expect("opening a note should create an editor");

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click_at("editor", point(px(1.), px(1.)), cx);
            // Move the cursor near the end, which (per the test above)
            // autoscrolls the view down to it.
            editor.update(cx, |editor, cx| {
                editor.buffer_mut().move_to_end(false);
                cx.notify();
            });
            window.render_frame(cx);
        })
        .unwrap();

        // Simulate the user then scrolling away from the cursor, back up
        // toward the top — same effect a mouse wheel would have.
        let scrolled_away = point(px(0.), px(0.));
        editor.read_with(cx, |editor, _| editor.scroll_handle().set_offset(scrolled_away));

        // A repaint with no cursor movement at all (e.g. the one a
        // mouse-wheel scroll itself triggers) must not fight the scroll
        // back toward the cursor.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();

        let offset = editor.read_with(cx, |editor, _| editor.scroll_handle().offset());
        assert_eq!(
            offset, scrolled_away,
            "a repaint with an unchanged cursor should not snap the scroll position back toward it"
        );
    }

    #[gpui_kit::test]
    fn linking_a_new_note_to_an_existing_note_shows_up_as_its_backlink(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "existing note").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
            window.render_frame(cx);

            // A brand-new note opens with its own editor ready immediately — no
            // click needed before it can be typed into.
            let editor = app.read(cx).editor.clone().expect("editor should be open");
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.input("Linking to [[Note A]].", cx);
            window.render_frame(cx);

            // Switch notes immediately after typing, with no time for the
            // editor's own `EditorEvent::Changed` autosave to have fired yet —
            // `open_note` must flush the outgoing note's text itself.
            window.click("Note A", cx);
            window.render_frame(cx);
            window.click("backlink-Untitled", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Untitled".to_string()));
    }

    #[gpui_kit::test]
    fn renamed_note_still_links_correctly_in_both_directions(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "existing note").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());

        // New note -> rename "Untitled" to "Hello" via the F2 dialog.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
            window.render_frame(cx);
            window.press("f2", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("rename-input", cx);
            window.press("end", cx);
            for _ in 0.."Untitled".len() {
                window.press("backspace", cx);
            }
            window.input("Hello", cx);
            window.click("confirm-rename", cx);
        })
        .unwrap();

        let renamed_id = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(renamed_id, Some("Hello".to_string()), "rename should have taken effect");
        assert!(dir.path().join("Hello.md").exists());
        assert!(!dir.path().join("Untitled.md").exists());

        // Type a link to Note A into the renamed note, then jump straight to Note A.
        // "Hello" already has its editor open, ready to type into.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let editor = app.read(cx).editor.clone().expect("editor should be open");
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.input("Linking to [[Note A]].", cx);
            window.render_frame(cx);

            window.click("Note A", cx);
            window.render_frame(cx);
            window.click("backlink-Hello", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Hello".to_string()), "Note A -> Hello backlink should navigate back");

        // And the reverse direction: from Note A, link to [[Hello]].
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note A", cx);
            window.render_frame(cx);
            let editor = app.read(cx).editor.clone().expect("editor should be open");
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.input(" Also linking to [[Hello]].", cx);
            window.render_frame(cx);

            window.click("Hello", cx);
            window.render_frame(cx);
            window.click("backlink-Note A", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note A".to_string()), "Hello -> Note A backlink should navigate back");
    }

    #[gpui_kit::test]
    fn new_note_button_creates_and_opens_a_note(cx: &mut TestAppContext) {
        let dir = temp_vault();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Untitled".to_string()));
        assert!(dir.path().join("Untitled.md").exists());
    }

    #[gpui_kit::test]
    fn editing_in_the_editor_autosaves_to_disk(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "old content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);

            let editor = app.read(cx).editor.clone().expect("editor should be open");
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.input("appended ", cx);
        })
        .unwrap();

        let on_disk = std::fs::read_to_string(dir.path().join("Note.md")).unwrap();
        assert_ne!(on_disk, "old content");
        assert!(on_disk.contains("appended"));
        assert!(on_disk.contains("old content"));
    }

    #[gpui_kit::test]
    fn opening_a_note_shows_its_editor_immediately(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "First paragraph.\n\nSecond paragraph.").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
        })
        .unwrap();

        let has_editor = app.read_with(cx, |app, _| app.editor.is_some());
        assert!(has_editor, "the editor should be ready for input as soon as a note is opened");

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("editor").is_some(), "the editor should be rendered");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_new_note_opens_with_an_editor_ready_for_input(cx: &mut TestAppContext) {
        let dir = temp_vault();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
        })
        .unwrap();

        let has_editor = app.read_with(cx, |app, _| app.editor.is_some());
        assert!(has_editor, "a brand-new note has nothing to click, so its editor must be ready right away");
    }

    #[gpui_kit::test]
    fn f2_hotkey_opens_rename_dialog_for_selected_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("f2", cx);
        })
        .unwrap();
        cx.run_until_parked();

        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();

        assert!(dialog_open);
    }

    #[gpui_kit::test]
    fn ctrl_p_opens_the_command_palette(cx: &mut TestAppContext) {
        // Deliberately nothing clicked first — this is the actual
        // regression case: a plain element-attached `.on_action` would
        // only ever fire via dispatch bubbling from whatever's currently
        // focused, and with *nothing* focused yet (right after opening the
        // app, before any click), that path collapses to empty. This is
        // why `JadeApp::open_command_palette` is wired through a global
        // `cx.on_action` instead — see its own doc comment.
        let dir = temp_vault();
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-p", cx);
        })
        .unwrap();
        cx.run_until_parked();

        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();
        assert!(dialog_open, "Ctrl+P should open the command palette");
    }

    #[gpui_kit::test]
    fn ctrl_p_works_regardless_of_current_focus(cx: &mut TestAppContext) {
        // The regression test for "global" specifically — `RenameNote`/
        // `DeleteNote`'s own hotkeys only fire while the sidebar's
        // `"NoteList"` key context has focus, but Ctrl+P's `KeyBinding` has
        // no context predicate at all, so it must still fire with the
        // editor focused instead.
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.click("editor", cx);
            window.render_frame(cx);
            window.press("ctrl-p", cx);
        })
        .unwrap();
        cx.run_until_parked();

        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();
        assert!(dialog_open, "Ctrl+P should open the palette even while the editor has focus");
    }

    #[gpui_kit::test]
    fn confirming_open_settings_closes_the_palette_and_opens_settings(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("ctrl-p", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // Only one item in the palette — Enter alone confirms it,
            // no need to click it first.
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("settings-panel").is_some(),
                "confirming \"Open Settings\" should have opened the Settings dialog"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicking_the_close_button_closes_the_settings_dialog(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("ctrl-p", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            // The fixed id `Dialog`'s own built-in close "x" button uses,
            // same way `AlertDialog`'s OK/Cancel are fixed "ok"/"cancel".
            window.click("close", cx);
        })
        .unwrap();
        cx.run_until_parked();

        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();
        assert!(!dialog_open, "clicking the x should close the Settings dialog");
    }

    #[gpui_kit::test]
    fn closing_a_dialog_does_not_leave_the_editor_mid_drag(cx: &mut TestAppContext) {
        // Guards against the bug class (not a proven reproduction of it —
        // see below): clicking a dialog positioned over the editor used to
        // also fire the editor's raw `MouseDownEvent` listener, since that
        // listener has no idea a dialog's backdrop is occluding it (GPUI's
        // `occlude()` only gates `.on_click`-style convenience wrappers,
        // never a raw `window.on_mouse_event`'s own `bounds.contains(...)`
        // check) — setting `is_dragging = true` with no matching `MouseUp`
        // reaching the editor to reset it (a dialog's own `Cancel` action,
        // dispatched synchronously from within that same click, poisons
        // `propagate_event` for the rest of that event's bubble, which the
        // editor's own later-registered `MouseUp` listener never gets to
        // see). Fixed in `text_editor::element::register_mouse_handling` by
        // bailing out of the Mouse-Down/Move listeners while any dialog is
        // open. Note: this specific test passes even *without* that fix —
        // the headless harness's `click()` dispatch apparently doesn't
        // reproduce the exact `propagate_event` leak a real windowed click
        // does (confirmed by temporarily reverting the fix and rerunning
        // this test alone) — so treat it as a basic sanity check, not a
        // proven regression guard for this exact bug.
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "hello world").unwrap();
        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("ctrl-p", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("close", cx);
        })
        .unwrap();
        cx.run_until_parked();

        // No button held — just moving the mouse over the editor.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.hover("editor", cx);
        })
        .unwrap();

        let editor = app.read_with(cx, |app, _| app.editor.clone()).expect("note should still be open");
        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert!(selection.is_none(), "moving the mouse with no button held must not start/extend a selection, got {selection:?}");
    }

    #[gpui_kit::test]
    fn pressing_enter_in_the_rename_dialog_confirms_it(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("f2", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("rename-input", cx);
            window.press("end", cx);
            for _ in 0.."Note".len() {
                window.press("backspace", cx);
            }
            window.input("Renamed", cx);
            // No click on "confirm-rename" — Enter alone must commit the rename.
            window.press("enter", cx);
        })
        .unwrap();

        let dialog_still_open =
            cx.update_window(window, |_, window, cx| window.has_active_dialog(cx)).unwrap();
        assert!(!dialog_still_open, "Enter should have closed the dialog");

        assert!(!dir.path().join("Note.md").exists());
        assert!(dir.path().join("Renamed.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Renamed".to_string()));
    }

    #[gpui_kit::test]
    fn backspace_hotkey_deletes_selected_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.press("backspace", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, None);
        assert!(!dir.path().join("Note.md").exists());
    }

    #[gpui_kit::test]
    fn backspace_in_the_editor_does_not_delete_the_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.click("Note", cx);
                window.render_frame(cx);

                let editor = app.read(cx).editor.clone().expect("editor should be open");
                editor.read(cx).focus_handle(cx).focus(window, cx);
                window.render_frame(cx);
                window.press("backspace", cx);
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();

        assert!(!dialog_open);
        assert!(dir.path().join("Note.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note".to_string()));
    }

    #[gpui_kit::test]
    fn context_menu_delete_removes_the_right_clicked_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "a").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "b").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note B", cx);
            window.render_frame(cx);
            window.right_click("Note A", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(1usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();

        assert!(!dir.path().join("Note A.md").exists());
        assert!(dir.path().join("Note B.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note B".to_string()));
    }

    #[gpui_kit::test]
    fn creating_a_folder_via_the_button_shows_it_in_the_sidebar(cx: &mut TestAppContext) {
        let dir = temp_vault();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-folder", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("create-folder-input", cx);
            window.input("Math", cx);
            window.click("confirm-create-folder", cx);
        })
        .unwrap();

        let has_folder = app.read_with(cx, |app, _| app.vault.folders.contains("Math"));
        assert!(has_folder, "vault should record the new folder");
        assert!(dir.path().join("Math").is_dir());

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("folder-Math").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn folder_context_menu_creates_a_nested_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(0usize, cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Math/Untitled".to_string()));
        assert!(dir.path().join("Math").join("Untitled.md").exists());
    }

    #[gpui_kit::test]
    fn folder_context_menu_creates_a_nested_subfolder(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(1usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("create-folder-input", cx);
            window.input("Algebra", cx);
            window.click("confirm-create-folder", cx);
        })
        .unwrap();

        let has_subfolder = app.read_with(cx, |app, _| app.vault.folders.contains("Math/Algebra"));
        assert!(has_subfolder);
        assert!(dir.path().join("Math").join("Algebra").is_dir());
    }

    #[gpui_kit::test]
    fn folder_context_menu_rename_moves_dir_and_nested_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "x + y").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(2usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("rename-folder-input", cx);
            window.press("end", cx);
            for _ in 0.."Math".len() {
                window.press("backspace", cx);
            }
            window.input("Mathematics", cx);
            window.click("confirm-rename-folder", cx);
        })
        .unwrap();

        assert!(!dir.path().join("Math").exists());
        assert!(dir.path().join("Mathematics").join("Algebra.md").exists());
        let has_folder = app.read_with(cx, |app, _| app.vault.folders.contains("Mathematics"));
        assert!(has_folder);
        let has_note = app.read_with(cx, |app, _| app.vault.note("Mathematics/Algebra").is_some());
        assert!(has_note);

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("folder-Mathematics").is_some());
            assert!(window.try_find("Mathematics/Algebra").is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn renaming_a_folder_reloads_its_currently_open_nested_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Math/Algebra", cx);
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(2usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("rename-folder-input", cx);
            window.press("end", cx);
            for _ in 0.."Math".len() {
                window.press("backspace", cx);
            }
            window.input("Mathematics", cx);
            window.click("confirm-rename-folder", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Mathematics/Algebra".to_string()));
        let has_editor = app.read_with(cx, |app, _| app.editor.is_some());
        assert!(has_editor, "the note should still be open after its folder was renamed");
    }

    #[gpui_kit::test]
    fn pressing_enter_in_the_rename_folder_dialog_confirms_it(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();

        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(2usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("rename-folder-input", cx);
            window.press("end", cx);
            for _ in 0.."Math".len() {
                window.press("backspace", cx);
            }
            window.input("Science", cx);
            // No click on "confirm-rename-folder" — Enter alone must commit the rename.
            window.press("enter", cx);
        })
        .unwrap();

        let dialog_still_open =
            cx.update_window(window, |_, window, cx| window.has_active_dialog(cx)).unwrap();
        assert!(!dialog_still_open, "Enter should have closed the dialog");
        assert!(dir.path().join("Science").is_dir());
    }

    #[gpui_kit::test]
    fn folder_context_menu_delete_removes_dir_and_contents(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();
        std::fs::write(dir.path().join("Other.md"), "untouched").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(3usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();

        assert!(!dir.path().join("Math").exists());
        assert!(dir.path().join("Other.md").exists());
        let has_folder = app.read_with(cx, |app, _| app.vault.folders.contains("Math"));
        assert!(!has_folder);
    }

    #[gpui_kit::test]
    fn deleting_a_folder_clears_selection_if_its_open_note_was_inside(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Math/Algebra", cx);
            window.render_frame(cx);
            window.right_click("folder-Math", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").within("items").click(3usize, cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, None);
        let has_editor = app.read_with(cx, |app, _| app.editor.is_some());
        assert!(!has_editor);
    }

    #[gpui_kit::test]
    fn clicking_a_folder_collapses_and_expands_its_children(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.path().to_path_buf());

        let visible_before = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("Math/Algebra").is_some()
            })
            .unwrap();
        assert!(visible_before, "note should be visible while folder is expanded");

        cx.update_window(window, |_, window, cx| {
            window.click("folder-Math", cx);
        })
        .unwrap();

        let visible_after_collapse = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("Math/Algebra").is_some()
            })
            .unwrap();
        assert!(!visible_after_collapse, "note should be hidden once folder is collapsed");

        cx.update_window(window, |_, window, cx| {
            window.click("folder-Math", cx);
        })
        .unwrap();

        let visible_after_expand = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("Math/Algebra").is_some()
            })
            .unwrap();
        assert!(visible_after_expand, "note should reappear once folder is expanded again");
    }

    #[gpui_kit::test]
    fn notes_in_a_subfolder_are_indented_further_than_top_level_notes(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Top Level.md"), "content").unwrap();
        std::fs::create_dir_all(dir.path().join("Math")).unwrap();
        std::fs::write(dir.path().join("Math").join("Algebra.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.path().to_path_buf());

        let (top_level_x, folder_x, nested_note_x) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                (
                    window.find("Top Level").bounds().origin.x,
                    window.find("folder-Math").bounds().origin.x,
                    window.find("Math/Algebra").bounds().origin.x,
                )
            })
            .unwrap();

        assert_eq!(
            folder_x, top_level_x,
            "a root-level folder should start at the same indent as a root-level note"
        );
        assert!(
            nested_note_x > top_level_x,
            "a note inside a folder should be indented further right than a top-level note (got {nested_note_x:?} vs {top_level_x:?})"
        );
    }

    #[gpui_kit::test]
    fn search_tab_filters_notes_by_title_and_content(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Apples.md"), "a fruit").unwrap();
        std::fs::write(dir.path().join("Carrots.md"), "a vegetable, tasty with apples").unwrap();
        std::fs::write(dir.path().join("Oranges.md"), "a different fruit").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("sidebar-tab-search", cx);
            window.render_frame(cx);
            window.click("search-input", cx);
            window.input("apple", cx);
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("search-result-Apples").is_some());
            assert!(window.try_find("search-result-Carrots").is_some());
            assert!(window.try_find("search-result-Oranges").is_none());
            window.click("search-result-Carrots", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Carrots".to_string()));
    }

    #[gpui_kit::test]
    fn backspace_while_typing_in_search_does_not_delete_the_selected_note(
        cx: &mut TestAppContext,
    ) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
            window.render_frame(cx);
            window.click("sidebar-tab-search", cx);
            window.render_frame(cx);
            window.click("search-input", cx);
            window.input("x", cx);
            window.press("backspace", cx);
        })
        .unwrap();
        cx.run_until_parked();

        let dialog_open = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.has_active_dialog(cx)
            })
            .unwrap();
        assert!(!dialog_open, "backspace in the search input must not open the delete dialog");

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note".to_string()));
        assert!(dir.path().join("Note.md").exists());
    }
}

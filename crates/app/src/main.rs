use std::path::PathBuf;

mod debug_server;
mod memory_probe;
#[cfg(test)]
mod test_alloc;
mod text_editor;

/// Test-build-only — see `test_alloc`'s own module doc for why. No effect
/// on release/`cargo run` (this whole item is `#[cfg(test)]`-gated).
#[cfg(test)]
#[global_allocator]
static GLOBAL_ALLOC: test_alloc::CountingAllocator = test_alloc::CountingAllocator;

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Theme, WindowExt};
use gpui_kit::*;
use jade_core::{Vault, search_targets};
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

/// Idle time after the last edit before the note is written to disk.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

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
    /// Pending debounced save; dropping it cancels the wait.
    autosave_task: Option<Task<()>>,
    _quit_subscription: Subscription,
    error: Option<String>,
    sidebar_focus: FocusHandle,
    collapsed_folders: std::collections::HashSet<String>,
    sidebar_tab: SidebarTab,
    search_input: Entity<InputState>,
    search_query: String,
    /// Search reads note files, so results are computed once per change
    /// (query or note text/set), never per frame: mutations set
    /// `search_stale` and `render` recomputes when the tab is showing.
    search_results: Vec<(String, String)>,
    search_stale: bool,
    /// Bumped per started search; a finished one whose generation is no
    /// longer current is dropped.
    search_generation: u64,
    /// The in-flight background search; dropping it cancels the wait.
    search_task: Option<Task<()>>,
    _search_subscription: Subscription,
}

/// The direct parent folder id of a note/folder id, or `None` at the vault root.
fn parent_of(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(parent, _)| parent)
}

impl JadeApp {
    fn new(vault_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar_focus = cx.focus_handle();

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search notes..."));
        let search_subscription = cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                this.search_stale = true;
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
            autosave_task: None,
            // Debounced saves can still be pending at quit.
            _quit_subscription: cx.on_app_quit(|this, cx| {
                this.save_open_editor(cx);
                async {}
            }),
            error,
            sidebar_focus,
            collapsed_folders: Default::default(),
            sidebar_tab: SidebarTab::default(),
            search_input,
            search_query: String::new(),
            search_results: Vec::new(),
            search_stale: true,
            search_generation: 0,
            search_task: None,
            _search_subscription: search_subscription,
        }
    }

    fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // Flush the outgoing note ourselves before swapping `self.editor` out: an
        // `EditorEvent::Changed` from the last keystroke could still be queued, and
        // without this, switching notes right after typing would drop it.
        self.flush_editor(cx);

        // The editor takes the only in-memory copy of the text.
        let Ok(text) = self.vault.read_note(&id) else {
            return;
        };
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
        self.autosave_task = None;
        self.persist_editor(&editor, &note_id, cx);
    }

    /// Writes the editor's text to the note's file straight from the buffer
    /// (no copy) and refreshes the note's cached links.
    fn persist_editor(&mut self, editor: &Entity<Editor>, note_id: &str, cx: &App) {
        let _ = self
            .vault
            .save_note(note_id, editor.read(cx).buffer().text());
        self.search_stale = true;
    }

    /// Schedules a save of the open editor, restarting the wait on every
    /// call: saving rescans the whole note for wikilinks and rewrites the
    /// file, which is far too much work to do per keystroke.
    fn autosave_editor(&mut self, cx: &mut Context<Self>) {
        self.autosave_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DEBOUNCE).await;
            this.update(cx, |this, cx| this.save_open_editor(cx)).ok();
        }));
    }

    /// Persists the open editor's current text to the vault, and re-renders
    /// so e.g. the backlinks panel reflects newly-typed wikilinks.
    fn save_open_editor(&mut self, cx: &mut Context<Self>) {
        self.autosave_task = None;
        let (Some(editor), Some(note_id)) = (self.editor.clone(), self.selected_id.clone()) else {
            return;
        };
        self.persist_editor(&editor, &note_id, cx);
        cx.notify();
    }

    fn create_note(&mut self, parent: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(id) = self.vault.create_note("Untitled", parent) {
            self.search_stale = true;
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

    fn rename_note(
        &mut self,
        id: &str,
        new_title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Flush first: renaming moves the note to a new id, so a flush keyed on
        // its *old* id (which the editor's edits are still relative to) has to
        // happen before that, not after.
        self.flush_editor(cx);

        let Ok(new_id) = self.vault.rename_note(id, new_title) else {
            return;
        };
        self.search_stale = true;
        if self.selected_id.as_deref() == Some(id) {
            self.open_note(new_id, window, cx);
        } else {
            cx.notify();
        }
    }

    fn delete_note(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.vault.delete_note(id).is_ok() {
            self.search_stale = true;
            if self.selected_id.as_deref() == Some(id) {
                self.selected_id = None;
                self.editor = None;
                self._editor_subscription = None;
            }
        }
        cx.notify();
    }

    fn rename_folder(
        &mut self,
        id: &str,
        new_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Flush the open note first (same reasoning as `open_note`'s own flush):
        // renaming the folder moves it to a new id, so a flush keyed on its *old*
        // id must happen before that, not after.
        self.flush_editor(cx);

        let Ok(new_id) = self.vault.rename_folder(id, new_name) else {
            return;
        };
        self.search_stale = true;

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
            self.search_stale = true;
            let prefix = format!("{id}/");
            self.collapsed_folders
                .retain(|f| f != id && !f.starts_with(&prefix));
            if self
                .selected_id
                .as_deref()
                .is_some_and(|sel| sel.starts_with(&prefix))
            {
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
            let title = self
                .vault
                .note(&id)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            Self::open_rename_dialog(id, title, cx.entity(), window, cx);
        }
    }

    fn delete_selected(&mut self, _: &DeleteNote, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_id.clone() {
            let title = self
                .vault
                .note(&id)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            Self::open_delete_dialog(id, title, cx.entity(), window, cx);
        }
    }

    fn commit_rename(
        id: &str,
        input: &Entity<InputState>,
        app: &Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
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

    fn commit_rename_folder(
        id: &str,
        input: &Entity<InputState>,
        app: &Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) {
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
                        .child(
                            Button::new("confirm-rename-folder")
                                .label("Rename")
                                .on_click(move |_, window, cx| {
                                    Self::commit_rename_folder(&id, &input, &app, window, cx);
                                    window.close_dialog(cx);
                                }),
                        ),
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
                        .child(
                            Button::new("confirm-create-folder")
                                .label("Create")
                                .on_click(move |_, window, cx| {
                                    Self::commit_create_folder(&parent, &input, &app, cx);
                                    window.close_dialog(cx);
                                }),
                        ),
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
        let mut folder_ids: Vec<&String> = self
            .vault
            .folders
            .iter()
            .filter(|id| parent_of(id) == parent)
            .collect();
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

    fn render_folder_row(
        &self,
        folder_id: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                menu.item(
                    PopupMenuItem::new("New note").on_click(move |_, window, cx| {
                        note_entity.update(cx, |app, cx| {
                            app.create_note(Some(&note_parent), window, cx);
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("New folder").on_click(move |_, window, cx| {
                        JadeApp::open_create_folder_dialog(
                            Some(folder_parent.clone()),
                            folder_entity.clone(),
                            window,
                            cx,
                        );
                    }),
                )
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

    /// Starts a background search if anything `search_results` depends on
    /// changed since the last call: searching reads every note file, so it
    /// must not block the UI thread. The previous results stay on screen
    /// until the new ones arrive.
    fn refresh_search(&mut self, cx: &mut Context<Self>) {
        if !self.search_stale {
            return;
        }
        self.search_stale = false;
        self.search_generation += 1;
        let generation = self.search_generation;
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            self.search_task = None;
            self.search_results.clear();
            return;
        }
        let targets = self.vault.search_targets();
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let mut results = cx
                .background_spawn(async move { search_targets(&targets, &query) })
                .await;
            results.sort_by(|a, b| a.1.cmp(&b.1));
            this.update(cx, |this, cx| {
                if this.search_generation == generation {
                    this.search_results = results;
                    this.search_task = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The "Search" sidebar tab: a live-filtered list of notes matching the
    /// query, by title or content substring (via `Vault::search`).
    fn render_search_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_search(cx);
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
            let results = self.search_results.clone();

            panel = panel.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(format!("Results ({})", results.len())),
            );
            if results.is_empty() {
                panel = panel.child(div().text_color(rgb(0x888888)).child("No matching notes."));
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
        let sidebar_content: AnyElement =
            match self.sidebar_tab {
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
                                .child(Button::new("new-note").label("New note").on_click(
                                    cx.listener(|this, _event, window, cx| {
                                        this.create_note(None, window, cx);
                                    }),
                                ))
                                .child(Button::new("new-folder").label("New folder").on_click(
                                    cx.listener(|_this, _event, window, cx| {
                                        let app_entity = cx.entity();
                                        JadeApp::open_create_folder_dialog(
                                            None, app_entity, window, cx,
                                        );
                                    }),
                                )),
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
            div()
                .p_4()
                .child(format!("Could not open vault: {err}"))
                .into_any_element()
        } else if let Some(note_id) = self.selected_id.clone() {
            let mut backlink_entries: Vec<(String, String)> = self
                .vault
                .backlinks()
                .get(&note_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|source_id| {
                    self.vault
                        .note(&source_id)
                        .map(|n| (source_id, n.title.clone()))
                })
                .collect();
            backlink_entries.sort_by(|a, b| a.1.cmp(&b.1));

            let mut backlinks_panel = div().flex().flex_col().gap_1().p_2().border_t_1().child(
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

    debug_server::start_if_enabled();
    memory_probe::start_if_enabled();

    // Registers the bundled Lucide icon SVGs (`Dialog`'s own close "x",
    // `Button::icon(...)`, etc.) with GPUI's asset loader — without this,
    // every `IconName` renders as empty (the button itself still paints,
    // just with nothing inside it), not just the close button specifically.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            install_fonts(cx);
            cx.bind_keys([
                KeyBinding::new("f2", RenameNote, Some("NoteList")),
                KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
                KeyBinding::new("tab", InsertTab, Some("TextEditor")),
                KeyBinding::new("ctrl-p", OpenCommandPalette, None),
            ]);
            bind_open_command_palette(cx);
            let (_, app) =
                gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                    cx.new(|cx| JadeApp::new(vault_path.clone(), window, cx))
                })
                .expect("failed to open window");
            debug_server::install(&app, cx);
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
        let Some(window_handle) = cx.windows().into_iter().next() else {
            return;
        };
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
        AnyWindowHandle, AppContext, Entity, Focusable, KeyBinding, TestAppContext, WindowOptions,
        point, px,
    };
    use std::path::PathBuf;
    use tempfile::{TempDir, tempdir};

    fn temp_vault() -> TempDir {
        tempdir().unwrap()
    }

    fn open_app(
        cx: &mut TestAppContext,
        vault_path: PathBuf,
    ) -> (AnyWindowHandle, Entity<JadeApp>) {
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

        let editor = app
            .read_with(cx, |app, _| app.editor.clone())
            .expect("opening a note should create an editor");

        let offset_before = editor.read_with(cx, |editor, _| editor.scroll_handle().offset());
        assert_eq!(
            offset_before.y,
            px(0.),
            "a freshly opened note should start scrolled to the top"
        );

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
    fn clicking_a_wrapped_lines_second_row_in_the_real_app_lands_further_into_the_text(
        cx: &mut TestAppContext,
    ) {
        let dir = temp_vault();
        let long: &'static str = Box::leak("a".repeat(2000).into_boxed_str());
        std::fs::write(dir.path().join("Long Line.md"), long).unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long Line", cx);
        })
        .unwrap();
        let editor = app
            .read_with(cx, |app, _| app.editor.clone())
            .expect("opening a note should create an editor");

        let line_height = cx
            .update_window(window, |_, window, _cx| window.line_height())
            .unwrap();

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

        let editor = app
            .read_with(cx, |app, _| app.editor.clone())
            .expect("opening a note should create an editor");

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
        editor.read_with(cx, |editor, _| {
            editor.scroll_handle().set_offset(scrolled_away)
        });

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
        assert_eq!(
            renamed_id,
            Some("Hello".to_string()),
            "rename should have taken effect"
        );
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
        assert_eq!(
            selected,
            Some("Hello".to_string()),
            "Note A -> Hello backlink should navigate back"
        );

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
        assert_eq!(
            selected,
            Some("Note A".to_string()),
            "Hello -> Note A backlink should navigate back"
        );
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

        // Debounced: nothing is written until the editor has been idle.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Note.md")).unwrap(),
            "old content"
        );
        cx.executor().advance_clock(crate::AUTOSAVE_DEBOUNCE);
        cx.run_until_parked();

        let on_disk = std::fs::read_to_string(dir.path().join("Note.md")).unwrap();
        assert_ne!(on_disk, "old content");
        assert!(on_disk.contains("appended"));
        assert!(on_disk.contains("old content"));
    }

    #[gpui_kit::test]
    fn opening_a_note_shows_its_editor_immediately(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(
            dir.path().join("Note.md"),
            "First paragraph.\n\nSecond paragraph.",
        )
        .unwrap();

        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note", cx);
        })
        .unwrap();

        let has_editor = app.read_with(cx, |app, _| app.editor.is_some());
        assert!(
            has_editor,
            "the editor should be ready for input as soon as a note is opened"
        );

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("editor").is_some(),
                "the editor should be rendered"
            );
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
        assert!(
            has_editor,
            "a brand-new note has nothing to click, so its editor must be ready right away"
        );
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
    fn opening_a_huge_file_does_not_take_several_seconds_to_render(cx: &mut TestAppContext) {
        // Regression test for a real user report: opening the 320K-line
        // stress fixture used to take ~10.1 SECONDS per `render_frame` —
        // `measure_document`/`paint_lines` shaped every line in the
        // document, every single paint, regardless of the viewport.
        // Viewport-bounded rendering (`text_editor::element`'s `LineLayout`/
        // `visible_line_range` — real shaping now only happens for the
        // handful of lines actually on screen) brought this down to ~1.4s
        // on the same file — a real ~7x win. This one render still pays a
        // full O(document) cost regardless (`RowHeightCache::rebuild`,
        // since the cache starts empty — the first paint after opening any
        // note always rebuilds it once), which is what this test still
        // measures; see `typing_in_a_huge_file_after_it_is_open_is_fast`
        // for the steady-state per-keystroke cost *after* that one-time
        // cost, which is what the 16.66ms/60fps bar actually applies to.
        // This threshold is deliberately loose (comfortably under 1.44s
        // measured, nowhere near the original 10s) — tight enough to catch
        // a real regression back toward "every line in the document gets
        // shaped again," loose enough not to be flaky on a slower CI
        // machine.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Big", cx);
            let elapsed = {
                let start = std::time::Instant::now();
                window.render_frame(cx);
                start.elapsed()
            };
            assert!(
                elapsed < std::time::Duration::from_secs(3),
                "rendering the stress fixture took {elapsed:?} — expected well under the old ~10.1s, ideally close to the ~1.4s this was fixed to"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn typing_in_a_huge_file_after_it_is_open_is_fast(cx: &mut TestAppContext) {
        // Diagnostic/regression test for the per-keystroke performance work
        // (incremental classification caching plus row-height caching, see
        // ARCHITECTURE.md's own `document_cache.rs`/`element.rs` bullets and
        // HISTORY.md for the measurement journey): once the file is open
        // (Part 1's viewport-bounded rendering already covers that cost), a
        // single keystroke's cost is what this measures — the actual
        // per-frame/per-keystroke steady-state cost the 16.66ms (60fps) bar
        // applies to, not cold-open. `edit` (buffer mutation + cache
        // patching) and `render` (now dominated by `RowHeightCache`'s own
        // cheap active-line sync, not a full `build_display_lines` sweep)
        // are reported separately rather than combined — see HISTORY.md for
        // why most of `edit`'s own cost traces to GPUI's own keystroke
        // dispatch, not this codebase.
        //
        // Deliberately *not* `window.input(...)` — `gpui_kit::test`'s own
        // `input_text` helper bundles a `render_frame` both before *and*
        // after dispatching each character, which would silently fold two
        // renders into whatever this measured as "the edit" itself. A
        // direct `dispatch_keystroke` isolates the actual edit path
        // (`EntityInputHandler::replace_text_in_range` ->
        // `EditorBuffer::insert` -> `DocumentCache::record_edit`) from the
        // render that follows it, so each can be attributed honestly.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();
        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Big", cx);
            window.render_frame(cx);
        })
        .unwrap();

        let editor = app.read_with(cx, |app, _| {
            app.editor.clone().expect("editor should be open")
        });
        cx.update_window(window, |_, window, cx| {
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);

            // Typing a character edits a single line deep inside a huge
            // document (no newline added/removed) — exactly the case
            // `DocumentCache`'s incremental patch functions exist for.
            let mut keystroke = gpui_kit::Keystroke::parse("x").unwrap();
            keystroke.key_char = Some("x".to_string());
            let edit_elapsed = {
                let start = std::time::Instant::now();
                window.dispatch_keystroke(keystroke, cx);
                start.elapsed()
            };
            let render_elapsed = {
                let start = std::time::Instant::now();
                window.render_frame(cx);
                start.elapsed()
            };
            eprintln!(
                "typing_in_a_huge_file_after_it_is_open_is_fast: edit={edit_elapsed:?} render={render_elapsed:?}"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn opening_and_repainting_a_huge_file_does_not_leak_per_paint_clones(cx: &mut TestAppContext) {
        // Diagnostic, not a correctness test — a real user reported ~189MB
        // RSS at startup vs. ~3xxMB with the 320K-line stress fixture open
        // (no edits made, just opened and looked at). `document_cache.rs`'s
        // own `block_context_list_tracking_allocation_cost_on_the_stress_
        // fixture` test accounts for part of the gap (`OpenBlock::List`'s
        // per-line clone, ~18MB/98K allocations) via a pure-Rust allocation
        // count. This test instead isolates a *different* candidate:
        // `render_editor`/`register_mouse_handling` clone the whole buffer
        // text (and `DocumentCache`'s six per-line `Vec`s) multiple times
        // on *every single paint*, purely to satisfy `window.on_mouse_
        // event`'s `'static` closure requirement — the question this test
        // actually answers is whether that's just wasteful *churn*
        // (allocate-then-free every paint, inflating the allocator's own
        // peak/high-water-mark but not actually leaking) or a genuine
        // *leak* (`current` bytes climbing without bound across repeated,
        // otherwise-identical repaints with no edits in between).
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();

        // Open the app and let it fully settle (window/font/UI-framework
        // one-time setup, plus the vault scan) *before* resetting the
        // counters — isolates the cost actually attributable to this one
        // document, matching the user's real comparison (an already-running
        // app, then opening the big file), rather than conflating it with
        // GPUI/font/window bootstrap cost that would happen regardless of
        // which note gets opened.
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);

            crate::test_alloc::reset();
            let before = crate::test_alloc::snapshot();
            window.click("Big", cx);
            window.render_frame(cx);
            let after_first_render = crate::test_alloc::snapshot();

            // Ten more repaints of the exact same, unchanged document —
            // nothing here should allocate anything that outlives the
            // paint it was allocated in.
            for _ in 0..10 {
                window.render_frame(cx);
            }
            let after_many_renders = crate::test_alloc::snapshot();

            eprintln!(
                "opening_and_repainting_a_huge_file_does_not_leak_per_paint_clones: \
                 opening this one file (app already running): current={} peak=+{} allocations={} | \
                 after 10 more unchanged renders: current={} peak=+{} allocations={}",
                after_first_render.current - before.current,
                after_first_render.peak - before.peak,
                after_first_render.count - before.count,
                after_many_renders.current - before.current,
                after_many_renders.peak - before.peak,
                after_many_renders.count - before.count,
            );

            // The real question: does `current` (live bytes right now, not
            // peak) keep growing across repeated identical repaints? A
            // generous slack (half the first render's own `current`) since
            // some GPUI-internal caches (shaped-glyph caches etc.) legitimately
            // grow a little as more of the document's distinct line shapes
            // get touched — this is a leak check, not a tight budget.
            assert!(
                after_many_renders.current < after_first_render.current + after_first_render.current / 2,
                "current live bytes grew from {} to {} across 10 unchanged repaints — looks like a real leak, not just per-paint churn",
                after_first_render.current,
                after_many_renders.current
            );
        })
        .unwrap();
    }

    /// Opens the stress fixture in a full app and focuses its editor.
    fn open_stress_fixture(
        cx: &mut TestAppContext,
        content: &str,
    ) -> (tempfile::TempDir, AnyWindowHandle, Entity<Editor>) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), content).unwrap();
        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Big", cx);
            window.render_frame(cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone().expect("editor open"));
        cx.update_window(window, |_, window, cx| {
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        (dir, window, editor)
    }

    #[gpui_kit::test]
    fn real_keystrokes_in_a_huge_file_keep_undo_memory_bounded(cx: &mut TestAppContext) {
        // Regression guard: keystrokes must go through `dispatch_keystroke`
        // (the real `replace_text_in_range` path), because calling
        // `EditorBuffer::insert` directly hid a bug where every keystroke
        // pushed a full-buffer undo snapshot (~11MB each on this fixture).
        const KEYSTROKES: usize = 150;
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, _editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            crate::test_alloc::reset();
            let before = crate::test_alloc::snapshot();
            let rss_before = crate::memory_probe::rss_kib();
            for _ in 0..KEYSTROKES {
                let mut keystroke = gpui_kit::Keystroke::parse("x").unwrap();
                keystroke.key_char = Some("x".to_string());
                window.dispatch_keystroke(keystroke, cx);
                window.render_frame(cx); // like the real app: one paint per key
            }
            window.render_frame(cx);
            let after = crate::test_alloc::snapshot();
            let retained = after.current.saturating_sub(before.current);
            eprintln!(
                "real_keystrokes_in_a_huge_file_keep_undo_memory_bounded: \
                 {KEYSTROKES} keystrokes retained {retained} bytes (buffer is {}), \
                 rss {rss_before:?} -> {:?} KiB",
                content.len(),
                crate::memory_probe::rss_kib()
            );
            // One coalesced burst costs about one buffer clone; allow a few
            // for caches. An uncoalesced run would retain KEYSTROKES clones.
            assert!(
                retained < 4 * content.len(),
                "retained {retained} bytes after {KEYSTROKES} keystrokes in a {}-byte \
                 buffer: undo coalescing is not working on the real typing path",
                content.len()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn real_keystrokes_in_a_huge_file_do_not_churn_the_whole_document(cx: &mut TestAppContext) {
        // Per-keystroke work must scale with the edit, not the document: a
        // whole-document copy per key (~11MB here) fragments the allocator
        // (RSS climbs while live heap stays flat) and costs CPU every key.
        const KEYSTROKES: usize = 50;
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, _editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            crate::test_alloc::reset();
            let started = std::time::Instant::now();
            for _ in 0..KEYSTROKES {
                let mut keystroke = gpui_kit::Keystroke::parse("x").unwrap();
                keystroke.key_char = Some("x".to_string());
                window.dispatch_keystroke(keystroke, cx);
                window.render_frame(cx);
            }
            let elapsed = started.elapsed();
            let total = crate::test_alloc::snapshot().total;
            let per_key = total / KEYSTROKES;
            eprintln!(
                "do_not_churn: {per_key} bytes allocated/keystroke (doc {}), {:?}/keystroke",
                content.len(),
                elapsed / KEYSTROKES as u32
            );
            assert!(
                per_key < content.len() / 4,
                "{per_key} bytes allocated per keystroke in a {}-byte document: \
                 something copies the whole document per key",
                content.len()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn first_keystroke_in_a_huge_file_does_not_reallocate_the_document(cx: &mut TestAppContext) {
        // A cloned note has no spare capacity, so the first insert used to
        // double the buffer (~11MB retained, an 11MB hole in the real app).
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, _editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            crate::test_alloc::reset();
            let before = crate::test_alloc::snapshot();
            let mut keystroke = gpui_kit::Keystroke::parse("x").unwrap();
            keystroke.key_char = Some("x".to_string());
            window.dispatch_keystroke(keystroke, cx);
            window.render_frame(cx);
            let after = crate::test_alloc::snapshot();
            let retained = after.current.saturating_sub(before.current);
            assert!(
                after.total < content.len() / 4 && retained < content.len() / 10,
                "first keystroke allocated {} and retained {retained} bytes in a \
                 {}-byte document",
                after.total,
                content.len()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn typing_a_character_in_a_huge_file_allocates_almost_nothing(cx: &mut TestAppContext) {
        // The edit path (buffer + cache patching, no render) used to copy
        // the callout list and every table per key: ~300KB each. Cost per
        // key must not scale with the number of callouts/tables.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, _editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            for i in 0..4 {
                let mut keystroke = gpui_kit::Keystroke::parse("x").unwrap();
                keystroke.key_char = Some("x".to_string());
                crate::test_alloc::reset();
                window.dispatch_keystroke(keystroke, cx);
                let snap = crate::test_alloc::snapshot();
                // The first key may still warm scratch buffers.
                if i > 0 {
                    assert!(
                        snap.total < 16 * 1024,
                        "key {i} allocated {} bytes in {} allocations",
                        snap.total,
                        snap.count
                    );
                }
                window.render_frame(cx);
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_paint_trace_reports_a_consistent_first_frame_over_the_socket(cx: &mut TestAppContext) {
        use crate::debug_server::test_client::{TestClient, start_test_server};
        use jade_debug_protocol::{Answer, Command, Event, Message, Topic};
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let sock = start_test_server();
        let mut client = TestClient::connect(&sock);
        assert_eq!(
            client.call(Command::Subscribe {
                topic: Topic::Paint
            }),
            Answer::Ok
        );

        let (_dir, window, _editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();

        let mut frames = Vec::new();
        while let Some(message) = client.try_recv(std::time::Duration::from_millis(300)) {
            if let Message::Event {
                event: Event::Paint {
                    consistent, trace, ..
                },
            } = message
            {
                frames.push((consistent, trace));
            }
        }
        std::fs::remove_file(&sock).unwrap();

        assert!(!frames.is_empty(), "no paint frames arrived");
        let (_, first) = &frames[0];
        assert!(first.contains("wrap_width=unknown"), "first frame: {first}");
        assert!(
            first.contains("callout"),
            "the fixture opens on callouts: {first}"
        );
        for (consistent, trace) in &frames {
            assert!(*consistent && !trace.contains("INCONSISTENT"), "{trace}");
        }
    }

    #[gpui_kit::test]
    fn commands_reach_the_main_thread_and_are_answered(cx: &mut TestAppContext) {
        use crate::debug_server::test_client::{TestClient, start_test_server};
        use jade_debug_protocol::{Answer, Command, Message, Query};
        let sock = start_test_server();
        // Server threads wake the main-thread pump; the test scheduler
        // otherwise rejects activity from threads it did not start.
        cx.executor().allow_parking();
        let dir = temp_vault();
        let (_window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update(|cx| crate::debug_server::install(&app, cx));
        let mut client = TestClient::connect(&sock);

        let req = client.send(Command::Query {
            what: Query::WindowSize,
        });
        let mut reply = None;
        for _ in 0..200 {
            cx.run_until_parked();
            if let Some(message) = client.try_recv(std::time::Duration::from_millis(10)) {
                reply = Some(message);
                break;
            }
        }
        std::fs::remove_file(&sock).unwrap();
        let Some(Message::Response { req: got, answer }) = reply else {
            panic!("the main thread never answered");
        };
        assert_eq!(got, req);
        let Answer::WindowSize { width, height } = answer else {
            panic!("expected a window size, got {answer:?}");
        };
        assert!(width > 0.0 && height > 0.0, "{width}x{height}");
    }

    /// Starts the debug server with its pump over `vault_path` and connects.
    fn open_app_with_debug_client(
        cx: &mut TestAppContext,
        vault_path: PathBuf,
    ) -> (
        crate::debug_server::test_client::TestClient,
        std::path::PathBuf,
        AnyWindowHandle,
        Entity<JadeApp>,
    ) {
        let sock = crate::debug_server::test_client::start_test_server();
        // Server threads wake the main-thread pump; the test scheduler
        // otherwise rejects activity from threads it did not start.
        cx.executor().allow_parking();
        let (window, app) = open_app(cx, vault_path);
        cx.update(|cx| crate::debug_server::install(&app, cx));
        let client = crate::debug_server::test_client::TestClient::connect(&sock);
        (client, sock, window, app)
    }

    /// Sends `command` and pumps the main thread until it is answered.
    fn call_on_main(
        cx: &mut TestAppContext,
        client: &mut crate::debug_server::test_client::TestClient,
        command: jade_debug_protocol::Command,
    ) -> jade_debug_protocol::Answer {
        use jade_debug_protocol::Message;
        let req = client.send(command);
        for _ in 0..300 {
            cx.run_until_parked();
            if let Some(Message::Response { req: got, answer }) =
                client.try_recv(std::time::Duration::from_millis(10))
                && got == req
            {
                return answer;
            }
        }
        panic!("the main thread never answered request {req}");
    }

    #[gpui_kit::test]
    fn typed_text_and_keys_over_the_socket_edit_the_real_editor(cx: &mut TestAppContext) {
        use jade_debug_protocol::{Answer, Command, Modifiers};
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "").unwrap();
        let (mut client, sock, window, app) =
            open_app_with_debug_client(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note A", cx);
            window.render_frame(cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone().expect("editor open"));
        cx.update_window(window, |_, window, cx| {
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        let text =
            |cx: &mut TestAppContext| editor.read_with(cx, |e, _| e.buffer().text().to_owned());

        let typed = Command::Type {
            text: "Hi There".into(),
        };
        assert_eq!(call_on_main(cx, &mut client, typed), Answer::Ok);
        assert_eq!(text(cx), "Hi There");

        let enter = Command::Key {
            key: "enter".into(),
            modifiers: Modifiers::default(),
        };
        assert_eq!(call_on_main(cx, &mut client, enter), Answer::Ok);
        assert_eq!(
            text(cx),
            "Hi There\n",
            "Enter must insert exactly one newline"
        );

        let multiline = Command::Type {
            text: "a\tb\nc".into(),
        };
        assert_eq!(call_on_main(cx, &mut client, multiline), Answer::Ok);
        assert_eq!(text(cx), "Hi There\na\tb\nc");

        let backspace = Command::Key {
            key: "backspace".into(),
            modifiers: Modifiers::default(),
        };
        assert_eq!(call_on_main(cx, &mut client, backspace), Answer::Ok);
        assert_eq!(text(cx), "Hi There\na\tb\n");

        let bad = Command::Key {
            key: String::new(),
            modifiers: Modifiers::default(),
        };
        assert!(matches!(
            call_on_main(cx, &mut client, bad),
            Answer::Error { .. }
        ));
        std::fs::remove_file(&sock).unwrap();
    }

    #[gpui_kit::test]
    fn a_click_over_the_socket_hits_the_element_under_the_point(cx: &mut TestAppContext) {
        use jade_debug_protocol::{Answer, Button, Command, Location, Modifiers, Point};
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.path().join("Note B.md"), "No outgoing links.").unwrap();
        let (mut client, sock, window, app) =
            open_app_with_debug_client(cx, dir.path().to_path_buf());
        let center = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.find("Note B").bounds().center()
            })
            .unwrap();
        let at = Point {
            x: center.x.into(),
            y: center.y.into(),
        };

        let click = Command::Click {
            at: Location::Point(at),
            button: Button::Left,
            count: 1,
            modifiers: Modifiers::default(),
        };
        assert_eq!(call_on_main(cx, &mut client, click), Answer::Ok);
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note B".to_string()));

        let named = Command::Click {
            at: Location::Target("Note A".into()),
            button: Button::Left,
            count: 1,
            modifiers: Modifiers::default(),
        };
        assert!(matches!(
            call_on_main(cx, &mut client, named),
            Answer::Error { .. }
        ));
        let too_many = Command::Click {
            at: Location::Point(at),
            button: Button::Left,
            count: 9,
            modifiers: Modifiers::default(),
        };
        assert!(matches!(
            call_on_main(cx, &mut client, too_many),
            Answer::Error { .. }
        ));
        std::fs::remove_file(&sock).unwrap();
    }

    #[gpui_kit::test]
    fn a_scroll_over_the_socket_scrolls_the_editor(cx: &mut TestAppContext) {
        use jade_debug_protocol::{Answer, Command, Modifiers, Point};
        let dir = temp_vault();
        let long = (0..400).map(|i| format!("line {i}\n")).collect::<String>();
        std::fs::write(dir.path().join("Long.md"), long).unwrap();
        let (mut client, sock, window, app) =
            open_app_with_debug_client(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Long", cx);
            window.render_frame(cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone().expect("editor open"));
        let before = editor.read_with(cx, |e, _| e.scroll_handle().offset());
        let size = cx
            .update_window(window, |_, window, _| window.viewport_size())
            .unwrap();
        let at = Point {
            x: f32::from(size.width) * 0.7,
            y: f32::from(size.height) * 0.5,
        };

        let scroll = Command::Scroll {
            at,
            dx: 0.0,
            dy: -300.0,
            modifiers: Modifiers::default(),
        };
        assert_eq!(call_on_main(cx, &mut client, scroll), Answer::Ok);
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        let after = editor.read_with(cx, |e, _| e.scroll_handle().offset());
        assert!(after.y < before.y, "scrolled down: {before:?} -> {after:?}");
        std::fs::remove_file(&sock).unwrap();
    }

    #[gpui_kit::test]
    fn the_editor_state_query_reflects_typing_and_selection(cx: &mut TestAppContext) {
        use jade_debug_protocol::{Answer, Command, EditorState, Modifiers, Query};
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "one\ntwo").unwrap();
        let (mut client, sock, window, app) =
            open_app_with_debug_client(cx, dir.path().to_path_buf());
        let query = Command::Query {
            what: Query::EditorState,
        };
        let closed = call_on_main(cx, &mut client, query.clone());
        assert_eq!(closed, Answer::EditorState(EditorState::default()));

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note A", cx);
            window.render_frame(cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone().expect("editor open"));
        cx.update_window(window, |_, window, cx| {
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        let Answer::EditorState(open) = call_on_main(cx, &mut client, query.clone()) else {
            panic!("expected editor state");
        };
        assert_eq!(open.note.as_deref(), Some("Note A"));
        assert_eq!((open.len, open.lines, open.cursor), (7, 2, 0));
        assert!(open.focused && open.selection.is_none());

        let typed = Command::Type { text: "X".into() };
        assert_eq!(call_on_main(cx, &mut client, typed), Answer::Ok);
        let select_all = Command::Key {
            key: "a".into(),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
        };
        assert_eq!(call_on_main(cx, &mut client, select_all), Answer::Ok);
        let Answer::EditorState(after) = call_on_main(cx, &mut client, query) else {
            panic!("expected editor state");
        };
        assert_eq!(after.len, 8);
        assert_eq!(after.selection, Some((0, 8)));
        std::fs::remove_file(&sock).unwrap();
    }

    #[gpui_kit::test]
    fn painted_frames_are_timed_and_streamed_over_the_socket(cx: &mut TestAppContext) {
        use jade_debug_protocol::{Answer, Command, Event, Message, Query, Topic};
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note A.md"), "one\ntwo").unwrap();
        let (mut client, sock, window, _app) =
            open_app_with_debug_client(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note A", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert_eq!(
            client.call(Command::Subscribe {
                topic: Topic::Frames
            }),
            Answer::Ok
        );
        let before = crate::debug_server::frames::current();
        cx.update_window(window, |_, window, cx| {
            for _ in 0..3 {
                window.render_frame(cx);
            }
        })
        .unwrap();

        let mut streamed = 0;
        while let Some(message) = client.try_recv(std::time::Duration::from_millis(100)) {
            if let Message::Event {
                event: Event::Frames(sample),
            } = message
            {
                assert!(sample.frame > before);
                streamed += 1;
            }
        }
        assert_eq!(streamed, 3, "one event per painted frame");

        let Answer::FrameTimes { frames } = client.call(Command::Query {
            what: Query::FrameTimes,
        }) else {
            panic!("expected frame times");
        };
        let fresh: Vec<_> = frames.iter().filter(|f| f.frame > before).collect();
        assert_eq!(fresh.len(), 3, "three painted frames: {frames:?}");
        assert!(fresh.windows(2).all(|w| w[0].frame + 1 == w[1].frame));
        assert!(fresh[1].interval_us > 0, "{fresh:?}");

        std::fs::remove_file(&sock).unwrap();
    }

    #[gpui_kit::test]
    fn first_paint_of_a_huge_note_needs_no_row_height_corrections(cx: &mut TestAppContext) {
        // A cold rebuild seeds wrapped lines with over-estimates. Callout
        // backgrounds are drawn before the lines, from those heights, so
        // painting must not still be correcting visible lines afterwards:
        // the bands would sit at the wrong height until the next repaint.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();
        let (window, app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Big", cx);
            window.render_frame(cx);
        })
        .unwrap();
        let editor = app.read_with(cx, |app, _| app.editor.clone().expect("editor open"));
        let late = editor.read_with(cx, |e, _| e.late_row_corrections());
        assert_eq!(
            late, 0,
            "paint corrected {late} visible lines after verification"
        );
    }

    #[gpui_kit::test]
    fn line_count_changing_edits_do_not_rebuild_the_document(cx: &mut TestAppContext) {
        // Enter, a line-joining Backspace and undo/redo of them used to recompute every
        // per-line cache (~165MB allocated, a 32MB peak, >150ms on the
        // stress fixture). Bytes allocated are asserted, not time: it is
        // deterministic.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            for key in [
                "x",
                "enter",
                "enter",
                "backspace",
                "backspace",
                "x",
                "enter",
                "ctrl-z",
                "ctrl-z",
                "ctrl-shift-z",
            ] {
                let len_before = editor.read(cx).buffer().text().len();
                crate::test_alloc::reset();
                let mut keystroke = gpui_kit::Keystroke::parse(key).unwrap();
                if key == "x" {
                    keystroke.key_char = Some("x".to_string());
                    window.dispatch_keystroke(keystroke, cx);
                } else {
                    // A bare key-down, as the real Linux platform sends
                    // (`dispatch_keystroke` would add a simulated text input
                    // that doubles Enter).
                    window.dispatch_event(
                        gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
                            keystroke,
                            is_held: false,
                            prefer_character_input: false,
                        }),
                        cx,
                    );
                }
                if key.starts_with("ctrl-") {
                    assert_ne!(
                        editor.read(cx).buffer().text().len(),
                        len_before,
                        "{key} did not change the text"
                    );
                }
                window.render_frame(cx);
                let snap = crate::test_alloc::snapshot();
                assert!(
                    snap.total < content.len() / 4 && snap.peak < snap.current + content.len() / 8,
                    "{key}: allocated {} bytes (peak {}) in a {}-byte document",
                    snap.total,
                    snap.peak,
                    content.len()
                );
            }
        })
        .unwrap();
    }

    #[test]
    fn opening_a_vault_does_not_buffer_a_whole_note() {
        // The scan extracts wikilinks line by line; reading each file whole
        // cost ~11 MiB of transient (then glibc-retained) heap for the
        // 11.8MB stress note.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();
        crate::test_alloc::reset();
        let before = crate::test_alloc::snapshot();
        let vault = jade_core::Vault::open(dir.path()).unwrap();
        let after = crate::test_alloc::snapshot();
        eprintln!(
            "vault open: peak {} KiB over baseline, live {} KiB",
            (after.peak - before.current) / 1024,
            (after.current - before.current) / 1024
        );
        assert!(
            after.peak < after.current + content.len() / 16,
            "vault open peaked {} bytes above its live {} for a {}-byte note",
            after.peak - before.current,
            after.current - before.current,
            content.len()
        );
        drop(vault);
    }

    #[gpui_kit::test]
    fn opening_a_huge_note_keeps_live_memory_bounded(cx: &mut TestAppContext) {
        // Live heap from before the vault is scanned to after the note is
        // open, in multiples of the note's size. The text must exist once
        // (the editor's buffer); the vault keeps none. Was ~3.9x with the
        // vault's own copy, ~5.5x before the per-line caches were slimmed.
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let dir = temp_vault();
        std::fs::write(dir.path().join("Big.md"), &content).unwrap();
        let before = crate::test_alloc::snapshot().current;
        let (window, _app) = open_app(cx, dir.path().to_path_buf());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let scanned = crate::test_alloc::snapshot().current.saturating_sub(before);
            window.click("Big", cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let opened = crate::test_alloc::snapshot().current.saturating_sub(before);
            eprintln!(
                "opening_a_huge_note: vault scan {:.1} MiB, with note open {:.1} MiB \
                 ({:.2}x the {}-byte note)",
                scanned as f64 / 1048576.0,
                opened as f64 / 1048576.0,
                opened as f64 / content.len() as f64,
                content.len()
            );
            assert!(
                scanned < content.len() / 4,
                "the vault scan retained {scanned} bytes for a {}-byte note: it must not \
                 keep note text",
                content.len()
            );
            assert!(
                opened < content.len() * 7 / 2,
                "opening the note added {opened} bytes for a {}-byte note",
                content.len()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn search_results_are_cached_until_something_changes(cx: &mut TestAppContext) {
        // Searching reads every note file, so `render` must not trigger it per
        // frame: results are recomputed only when marked stale, and off the
        // UI thread.
        let dir = temp_vault();
        std::fs::write(dir.path().join("Note.md"), "alpha").unwrap();
        let (_window, app) = open_app(cx, dir.path().to_path_buf());
        let refresh = |cx: &mut TestAppContext| {
            app.update(cx, |app, cx| app.refresh_search(cx));
            cx.run_until_parked();
        };
        app.update(cx, |app, _| app.search_query = "alpha".to_string());
        refresh(cx);
        assert_eq!(app.read_with(cx, |a, _| a.search_results.len()), 1);

        std::fs::write(dir.path().join("Note.md"), "beta").unwrap();
        refresh(cx);
        assert_eq!(
            app.read_with(cx, |a, _| a.search_results.len()),
            1,
            "cached: no re-read when not stale"
        );

        app.update(cx, |app, _| app.search_stale = true);
        refresh(cx);
        assert!(
            app.read_with(cx, |a, _| a.search_results.is_empty()),
            "stale: re-read from disk"
        );
    }

    #[gpui_kit::test]
    fn a_superseded_search_does_not_overwrite_newer_results(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.path().join("Alpha.md"), "x").unwrap();
        std::fs::write(dir.path().join("Beta.md"), "x").unwrap();
        let (_window, app) = open_app(cx, dir.path().to_path_buf());
        // Two searches started back to back; only the second may land.
        app.update(cx, |app, cx| {
            app.search_query = "alpha".to_string();
            app.refresh_search(cx);
            app.search_query = "beta".to_string();
            app.search_stale = true;
            app.refresh_search(cx);
        });
        cx.run_until_parked();
        let titles = app.read_with(cx, |a, _| {
            a.search_results
                .iter()
                .map(|r| r.1.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(titles, ["Beta"]);
    }

    #[gpui_kit::test]
    fn scrolling_through_a_huge_file_does_not_leak(cx: &mut TestAppContext) {
        // Logical heap (`test_alloc`) is asserted; RSS is only reported,
        // because it depends on the allocator's own trim heuristics and a
        // hard threshold would be flaky.
        const SCROLL_STEPS: usize = 200;
        let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
        let Ok(content) = std::fs::read_to_string(fixture_path) else {
            eprintln!("stress fixture not found at {fixture_path:?}, skipping");
            return;
        };
        let (_dir, window, editor) = open_stress_fixture(cx, &content);
        cx.update_window(window, |_, window, cx| {
            let handle = editor.read(cx).scroll_handle().clone();
            let max_y = f32::from(handle.max_offset().y.abs());
            let rss_before = crate::memory_probe::rss_kib();
            let before = crate::test_alloc::snapshot();
            for step in 0..SCROLL_STEPS {
                let y = max_y * step as f32 / SCROLL_STEPS as f32;
                handle.set_offset(point(px(0.), px(-y)));
                window.render_frame(cx);
            }
            let after = crate::test_alloc::snapshot();
            let rss_after = crate::memory_probe::rss_kib();
            eprintln!(
                "scrolling_through_a_huge_file_does_not_leak: max_scroll={max_y}px \
                 heap {} -> {} bytes, rss {rss_before:?} -> {rss_after:?} KiB",
                before.current, after.current
            );
            assert!(max_y > 0.0, "fixture should be scrollable");
            assert!(
                after.current < before.current + before.current / 2,
                "heap grew from {} to {} across {SCROLL_STEPS} scroll steps",
                before.current,
                after.current
            );
        })
        .unwrap();
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
        assert!(
            dialog_open,
            "Ctrl+P should open the palette even while the editor has focus"
        );
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
        assert!(
            !dialog_open,
            "clicking the x should close the Settings dialog"
        );
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

        let editor = app
            .read_with(cx, |app, _| app.editor.clone())
            .expect("note should still be open");
        let selection = editor.read_with(cx, |editor, _| editor.buffer().selection());
        assert!(
            selection.is_none(),
            "moving the mouse with no button held must not start/extend a selection, got {selection:?}"
        );
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

        let dialog_still_open = cx
            .update_window(window, |_, window, cx| window.has_active_dialog(cx))
            .unwrap();
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
            window
                .within("popup-menu")
                .within("items")
                .click(1usize, cx);
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
            window
                .within("popup-menu")
                .within("items")
                .click(0usize, cx);
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
            window
                .within("popup-menu")
                .within("items")
                .click(1usize, cx);
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
            window
                .within("popup-menu")
                .within("items")
                .click(2usize, cx);
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
            window
                .within("popup-menu")
                .within("items")
                .click(2usize, cx);
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
        assert!(
            has_editor,
            "the note should still be open after its folder was renamed"
        );
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
            window
                .within("popup-menu")
                .within("items")
                .click(2usize, cx);
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

        let dialog_still_open = cx
            .update_window(window, |_, window, cx| window.has_active_dialog(cx))
            .unwrap();
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
            window
                .within("popup-menu")
                .within("items")
                .click(3usize, cx);
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
            window
                .within("popup-menu")
                .within("items")
                .click(3usize, cx);
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
        assert!(
            visible_before,
            "note should be visible while folder is expanded"
        );

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
        assert!(
            !visible_after_collapse,
            "note should be hidden once folder is collapsed"
        );

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
        assert!(
            visible_after_expand,
            "note should reappear once folder is expanded again"
        );
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
        std::fs::write(
            dir.path().join("Carrots.md"),
            "a vegetable, tasty with apples",
        )
        .unwrap();
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
    fn backspace_while_typing_in_search_does_not_delete_the_selected_note(cx: &mut TestAppContext) {
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
        assert!(
            !dialog_open,
            "backspace in the search input must not open the delete dialog"
        );

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note".to_string()));
        assert!(dir.path().join("Note.md").exists());
    }
}

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::WindowExt;
use gpui_kit::*;
use jade_core::Vault;

actions!(jade, [RenameNote, DeleteNote]);

struct JadeApp {
    vault: Vault,
    selected_id: Option<String>,
    editor: Option<Entity<TextareaState>>,
    _editor_subscription: Option<Subscription>,
    error: Option<String>,
    sidebar_focus: FocusHandle,
    collapsed_folders: std::collections::HashSet<String>,
}

/// The direct parent folder id of a note/folder id, or `None` at the vault root.
fn parent_of(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(parent, _)| parent)
}

impl JadeApp {
    fn new(vault_path: PathBuf, cx: &mut Context<Self>) -> Self {
        let sidebar_focus = cx.focus_handle();
        match Vault::open(&vault_path) {
            Ok(vault) => Self {
                vault,
                selected_id: None,
                editor: None,
                _editor_subscription: None,
                error: None,
                sidebar_focus,
                collapsed_folders: Default::default(),
            },
            Err(err) => Self {
                vault: Vault { root: vault_path, notes: Default::default(), folders: Default::default() },
                selected_id: None,
                editor: None,
                _editor_subscription: None,
                error: Some(err.to_string()),
                sidebar_focus,
                collapsed_folders: Default::default(),
            },
        }
    }

    fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // Flush the outgoing note's latest text ourselves: its `InputEvent::Change`
        // autosave can still be pending (GPUI dispatches it on a delay), and without
        // this, switching notes right after typing drops whatever hasn't flushed yet.
        if let (Some(old_id), Some(old_editor)) = (self.selected_id.clone(), self.editor.clone()) {
            let text = old_editor.read(cx).value().to_string();
            let _ = self.vault.update_note_content(&old_id, text);
        }

        let Some(note) = self.vault.note(&id) else {
            return;
        };
        let content = note.content.clone();

        let editor = cx.new(|cx| TextareaState::new(window, cx).default_value(content));
        let subscription = cx.subscribe(&editor, move |this, editor, event, cx| {
            if matches!(event, InputEvent::Change) {
                let text = editor.read(cx).value().to_string();
                if let Some(id) = this.selected_id.clone() {
                    let _ = this.vault.update_note_content(&id, text);
                }
                cx.notify();
            }
        });

        self.selected_id = Some(id);
        self.editor = Some(editor);
        self._editor_subscription = Some(subscription);
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
}

impl Render for JadeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_rows = self.render_tree_level(None, 0, cx);

        let sidebar = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(220.))
            .p_2()
            .border_r_1()
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
                                JadeApp::open_create_folder_dialog(None, app_entity, window, cx);
                            })),
                    ),
            )
            .children(tree_rows);

        let content: AnyElement = if let Some(err) = &self.error {
            div().p_4().child(format!("Could not open vault: {err}")).into_any_element()
        } else if let Some(editor) = &self.editor {
            let mut backlink_entries: Vec<(String, String)> = self
                .selected_id
                .as_ref()
                .map(|id| self.vault.backlinks().get(id).cloned().unwrap_or_default())
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

            div()
                .flex()
                .flex_col()
                .size_full()
                .p_4()
                .child(Textarea::new(editor).flex_1())
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

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("f2", RenameNote, Some("NoteList")),
            KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
        ]);
        gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
            cx.new(|cx| JadeApp::new(vault_path.clone(), cx))
        })
        .expect("failed to open window");
    });
}

// Deliberately not `use super::*;` / `use gpui_kit::*;` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows the plain `#[test]` its own expansion relies on and recurses forever.
#[cfg(test)]
mod tests {
    use super::{DeleteNote, JadeApp, RenameNote};
    use gpui_kit::component::WindowExt;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Entity, Focusable, KeyBinding, TestAppContext, WindowOptions,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_vault() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("jade_app_test_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open_app(cx: &mut TestAppContext, vault_path: PathBuf) -> (AnyWindowHandle, Entity<JadeApp>) {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("f2", RenameNote, Some("NoteList")),
                KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
            ]);
            gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
                cx.new(|cx| JadeApp::new(vault_path.clone(), cx))
            })
            .expect("open test window")
        })
    }

    #[gpui_kit::test]
    fn clicking_sidebar_note_opens_it(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.join("Note B.md"), "No outgoing links.").unwrap();

        let (window, app) = open_app(cx, dir.clone());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note B", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note B".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn clicking_backlink_navigates_to_source_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note A.md"), "Links to [[Note B]].").unwrap();
        std::fs::write(dir.join("Note B.md"), "No outgoing links.").unwrap();

        let (window, app) = open_app(cx, dir.clone());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("Note B", cx);
            window.render_frame(cx);
            window.click("backlink-Note A", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note A".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn linking_a_new_note_to_an_existing_note_shows_up_as_its_backlink(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note A.md"), "existing note").unwrap();

        let (window, app) = open_app(cx, dir.clone());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
            window.render_frame(cx);

            let editor = app.read(cx).editor.clone().expect("editor should be open");
            editor.read(cx).focus_handle(cx).focus(window, cx);
            window.render_frame(cx);
            window.input("Linking to [[Note A]].", cx);
            window.render_frame(cx);

            // Switch notes immediately after typing, with no time for GPUI's
            // own (delayed) `InputEvent::Change` autosave to have fired yet —
            // `open_note` must flush the outgoing note's text itself.
            window.click("Note A", cx);
            window.render_frame(cx);
            window.click("backlink-Untitled", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Untitled".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn renamed_note_still_links_correctly_in_both_directions(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note A.md"), "existing note").unwrap();

        let (window, app) = open_app(cx, dir.clone());

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
        assert!(dir.join("Hello.md").exists());
        assert!(!dir.join("Untitled.md").exists());

        // Type a link to Note A into the renamed note, then jump straight to Note A.
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

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn new_note_button_creates_and_opens_a_note(cx: &mut TestAppContext) {
        let dir = temp_vault();

        let (window, app) = open_app(cx, dir.clone());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("new-note", cx);
        })
        .unwrap();

        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Untitled".to_string()));
        assert!(dir.join("Untitled.md").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn editing_in_the_textarea_autosaves_to_disk(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note.md"), "old content").unwrap();

        let (window, app) = open_app(cx, dir.clone());
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

        let on_disk = std::fs::read_to_string(dir.join("Note.md")).unwrap();
        assert_ne!(on_disk, "old content");
        assert!(on_disk.contains("appended"));
        assert!(on_disk.contains("old content"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn f2_hotkey_opens_rename_dialog_for_selected_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.clone());
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

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn pressing_enter_in_the_rename_dialog_confirms_it(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.clone());
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

        assert!(!dir.join("Note.md").exists());
        assert!(dir.join("Renamed.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Renamed".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn backspace_hotkey_deletes_selected_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.clone());
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
        assert!(!dir.join("Note.md").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn backspace_in_the_editor_does_not_delete_the_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note.md"), "content").unwrap();

        let (window, app) = open_app(cx, dir.clone());
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
        assert!(dir.join("Note.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn context_menu_delete_removes_the_right_clicked_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Note A.md"), "a").unwrap();
        std::fs::write(dir.join("Note B.md"), "b").unwrap();

        let (window, app) = open_app(cx, dir.clone());
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

        assert!(!dir.join("Note A.md").exists());
        assert!(dir.join("Note B.md").exists());
        let selected = app.read_with(cx, |app, _| app.selected_id.clone());
        assert_eq!(selected, Some("Note B".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn creating_a_folder_via_the_button_shows_it_in_the_sidebar(cx: &mut TestAppContext) {
        let dir = temp_vault();

        let (window, app) = open_app(cx, dir.clone());
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
        assert!(dir.join("Math").is_dir());

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("folder-Math").is_some());
        })
        .unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn folder_context_menu_creates_a_nested_note(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.join("Math")).unwrap();

        let (window, app) = open_app(cx, dir.clone());
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
        assert!(dir.join("Math").join("Untitled.md").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn folder_context_menu_creates_a_nested_subfolder(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.join("Math")).unwrap();

        let (window, app) = open_app(cx, dir.clone());
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
        assert!(dir.join("Math").join("Algebra").is_dir());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn clicking_a_folder_collapses_and_expands_its_children(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::create_dir_all(dir.join("Math")).unwrap();
        std::fs::write(dir.join("Math").join("Algebra.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.clone());

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

        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui_kit::test]
    fn notes_in_a_subfolder_are_indented_further_than_top_level_notes(cx: &mut TestAppContext) {
        let dir = temp_vault();
        std::fs::write(dir.join("Top Level.md"), "content").unwrap();
        std::fs::create_dir_all(dir.join("Math")).unwrap();
        std::fs::write(dir.join("Math").join("Algebra.md"), "content").unwrap();

        let (window, _app) = open_app(cx, dir.clone());

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

        std::fs::remove_dir_all(&dir).ok();
    }
}

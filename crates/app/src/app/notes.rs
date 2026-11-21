//! Creating, renaming and deleting notes and folders, and the tree selection.

#[allow(unused_imports)]
use super::*;

impl JadeApp {
    pub(super) fn create_note(
        &mut self,
        parent: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(id) = self.vault.create_note("Untitled", parent) {
            self.search_stale = true;
            self.open_note(id, window, cx);
            self.refresh_links(cx);
            self.sidebar_focus.focus(window, cx);
        }
    }

    pub(super) fn create_folder(
        &mut self,
        name: &str,
        parent: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let _ = self.vault.create_folder(name, parent.as_deref());
        cx.notify();
    }

    pub(super) fn toggle_folder(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        if !self.collapsed_folders.remove(folder_id) {
            self.collapsed_folders.insert(folder_id.to_string());
        }
        cx.notify();
    }

    pub(super) fn rename_note(
        &mut self,
        id: &str,
        new_title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Save first: renaming moves the note to a new id, so a save keyed on
        // its *old* id (which the editors' edits are still relative to) has to
        // happen before that, not after.
        self.persist_all_tabs(cx);

        // Every link to the note, in every note, is rewritten to its new name.
        let Ok(renamed) = self.vault.rename_note_updating_links(id, new_title) else {
            return;
        };
        self.search_stale = true;
        // The renamed note moved; each other rewritten note changed on disk
        // under its open tab, so both are reloaded.
        self.retarget_tabs(
            |old| {
                if old == id {
                    Some(renamed.new_id.clone())
                } else {
                    renamed.rewritten.iter().find(|n| *n == old).cloned()
                }
            },
            window,
            cx,
        );
        self.refresh_links(cx);
    }

    pub(super) fn delete_note(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.vault.delete_note(id).is_ok() {
            self.search_stale = true;
            if let Some(ix) = self.tab_of(id) {
                self.remove_tab(ix);
            }
            self.refresh_links(cx);
        }
        cx.notify();
    }

    pub(super) fn rename_folder(
        &mut self,
        id: &str,
        new_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Save the open notes first (same reasoning as `rename_note`).
        self.persist_all_tabs(cx);

        // Links into the folder, from every note, follow its new name.
        let Ok(renamed) = self.vault.rename_folder_updating_links(id, new_name) else {
            return;
        };
        let new_id = renamed.new_id;
        self.search_stale = true;

        if self.collapsed_folders.remove(id) {
            self.collapsed_folders.insert(new_id.clone());
        }

        let prefix = format!("{id}/");
        // Notes inside moved; other rewritten notes changed on disk under
        // their open tabs. Both reload.
        self.retarget_tabs(
            |old| match old.strip_prefix(&prefix) {
                Some(rest) => Some(format!("{new_id}/{rest}")),
                None => renamed.rewritten.iter().find(|n| *n == old).cloned(),
            },
            window,
            cx,
        );
        self.refresh_links(cx);
    }

    pub(super) fn delete_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.vault.delete_folder(id).is_ok() {
            self.search_stale = true;
            let prefix = format!("{id}/");
            self.collapsed_folders
                .retain(|f| f != id && !f.starts_with(&prefix));
            for ix in (0..self.tabs.len()).rev() {
                if self.tabs[ix].note_id.starts_with(&prefix) {
                    self.remove_tab(ix);
                }
            }
            self.refresh_links(cx);
        }
        cx.notify();
    }

    pub(super) fn rename_selected(
        &mut self,
        _: &RenameNote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected_id().map(str::to_string) {
            let title = self
                .vault
                .note(&id)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            Self::open_rename_dialog(id, title, cx.entity(), window, cx);
        }
    }

    pub(super) fn delete_selected(
        &mut self,
        _: &DeleteNote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected_id().map(str::to_string) {
            let title = self
                .vault
                .note(&id)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            Self::open_delete_dialog(id, title, cx.entity(), window, cx);
        }
    }

    pub(super) fn commit_rename(
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

    pub(super) fn commit_rename_folder(
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

    pub(super) fn commit_create_folder(
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
}

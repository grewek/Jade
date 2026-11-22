//! Open notes: one tab each, their editors, saving, and link following.
//! [`Tabs`] holds the list and its invariants; the `impl JadeApp` below
//! orchestrates it with the vault and the window.

#[allow(unused_imports)]
use super::*;

/// One open note. The editor is Jade's own `text_editor::Editor` (not
/// `gpui-component`'s `Textarea`); each tab keeps its own, so cursor, scroll and
/// undo survive switching tabs.
pub(crate) struct Tab {
    pub(crate) note_id: String,
    pub(super) editor: Entity<Editor>,
    /// Set by the editor's `Changed` event, cleared when the autosave writes it.
    pub(super) dirty: bool,
    pub(super) _subscription: Subscription,
}

/// The open notes, at most one tab per note, and which one is shown. The list
/// and the active index only change through these methods, so the index always
/// points at a tab (or is `None` when there is none).
pub(crate) struct Tabs {
    list: Vec<Tab>,
    active: Option<usize>,
    /// Horizontal scroll of the tab bar, so the active tab can be brought into view.
    scroll: ScrollHandle,
    /// Pending debounced save of the dirty tabs; dropping it cancels the wait.
    autosave: Option<Task<()>>,
}

impl Tabs {
    pub(super) fn new() -> Self {
        Self {
            list: Vec::new(),
            active: None,
            scroll: ScrollHandle::new(),
            autosave: None,
        }
    }

    /// The tab being shown.
    pub(crate) fn active(&self) -> Option<&Tab> {
        self.list.get(self.active?)
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub(super) fn index_of(&self, note_id: &str) -> Option<usize> {
        self.list.iter().position(|tab| tab.note_id == note_id)
    }

    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, Tab> {
        self.list.iter()
    }

    pub(super) fn iter_mut(&mut self) -> std::slice::IterMut<'_, Tab> {
        self.list.iter_mut()
    }

    /// Adds a tab at the end and shows it; returns its index.
    pub(super) fn push(&mut self, tab: Tab) -> usize {
        self.list.push(tab);
        let ix = self.list.len() - 1;
        self.active = Some(ix);
        self.scroll.scroll_to_item(ix);
        ix
    }

    /// Shows tab `ix` and scrolls the bar to it; `false` when there is none.
    pub(super) fn activate(&mut self, ix: usize) -> bool {
        if ix >= self.list.len() {
            return false;
        }
        self.active = Some(ix);
        self.scroll.scroll_to_item(ix);
        true
    }

    /// Drops the tab without saving (its note is gone, or already saved). The
    /// right neighbour becomes active if the active tab goes, else the left.
    pub(super) fn remove(&mut self, ix: usize) {
        self.list.remove(ix);
        self.active = active_after_remove(self.active, ix, self.list.len());
    }

    pub(super) fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// Keeps (or, with `None`, cancels) the pending autosave.
    pub(super) fn set_autosave(&mut self, task: Option<Task<()>>) {
        self.autosave = task;
    }
}

impl std::ops::Index<usize> for Tabs {
    type Output = Tab;

    fn index(&self, ix: usize) -> &Tab {
        &self.list[ix]
    }
}

impl std::ops::IndexMut<usize> for Tabs {
    fn index_mut(&mut self, ix: usize) -> &mut Tab {
        &mut self.list[ix]
    }
}

/// The active index after tab `removed` went, `len` tabs being left.
fn active_after_remove(active: Option<usize>, removed: usize, len: usize) -> Option<usize> {
    match active {
        _ if len == 0 => None,
        Some(active) if active == removed => Some(removed.min(len - 1)),
        Some(active) if active > removed => Some(active - 1),
        other => other,
    }
}

impl JadeApp {
    /// The tab being shown.
    pub(super) fn active(&self) -> Option<&Tab> {
        self.tabs.active()
    }

    pub(crate) fn selected_id(&self) -> Option<&str> {
        self.active().map(|tab| tab.note_id.as_str())
    }

    pub(crate) fn editor(&self) -> Option<&Entity<Editor>> {
        self.active().map(|tab| &tab.editor)
    }

    pub(super) fn tab_of(&self, note_id: &str) -> Option<usize> {
        self.tabs.index_of(note_id)
    }

    /// Reads `id` from the vault into a fresh editor; the editor takes the only
    /// in-memory copy of the text.
    pub(super) fn new_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Tab> {
        let text = self.vault.read_note(id).ok()?;
        let editor = cx.new(|cx| Editor::new(text, cx));
        let root = self.vault.root.clone();
        editor.update(cx, |editor, cx| {
            editor.set_image_context(root, id.to_string(), cx)
        });
        self.resolve_links_in(&editor, id, false, cx);
        let subscription =
            cx.subscribe_in(
                &editor,
                window,
                |this, editor, event, window, cx| match event {
                    EditorEvent::Changed => this.note_changed(editor.entity_id(), cx),
                    EditorEvent::OpenLink {
                        kind,
                        dest,
                        heading,
                    } => this.follow_link(*kind, dest, heading.as_deref(), window, cx),
                },
            );
        Some(Tab {
            note_id: id.to_string(),
            editor,
            dirty: false,
            _subscription: subscription,
        })
    }

    /// Follows a clicked link: notes open in a tab, web and mail links go to
    /// the system handler, other files inside the vault open with their
    /// default app. Anything unresolved, outside the vault or with another
    /// scheme is ignored.
    /// The active tab's editor, read back by tests.
    #[cfg(test)]
    pub(super) fn active_editor_for_test<'a>(&'a self, cx: &'a App) -> &'a Editor {
        self.editor().expect("a note is open").read(cx)
    }

    pub(super) fn follow_link(
        &mut self,
        kind: LinkKind,
        dest: &str,
        heading: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.active().map(|tab| tab.note_id.clone()) else {
            return;
        };
        // A wikilink carries its `#heading` apart from its target; the rest
        // write it after the path (percent-escaped; `?query` is not one).
        let heading = heading.map(str::to_owned).or_else(|| {
            let (_, fragment) = dest.split_once('#')?;
            jade_core::percent_decode(fragment)
        });
        match self.vault.link_destination(&from, kind, dest) {
            Destination::Note(id) => self.open_note_at(id, heading.as_deref(), window, cx),
            // `[x](#heading)` points into the note it is written in.
            Destination::SameNote => {
                if let Some(heading) = heading {
                    self.jump_to_heading(&heading, cx);
                }
            }
            Destination::File(path) => cx.open_with_system(&path),
            Destination::External("http" | "https" | "mailto") => cx.open_url(dest),
            // Another scheme, a missing target or one outside the vault.
            Destination::External(_) | Destination::Missing => {}
        }
    }

    /// Opens note `id` and, if given, moves to its heading.
    pub(super) fn open_note_at(
        &mut self,
        id: String,
        heading: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_note(id, window, cx);
        if let Some(heading) = heading {
            self.jump_to_heading(heading, cx);
        }
    }

    /// Moves the active note's cursor to the start of zero-based `line`.
    pub(super) fn jump_to_line(&self, line: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| editor.jump_to_line(line, cx));
        }
    }

    /// Moves the active note's cursor to `heading`, if it has one.
    pub(super) fn jump_to_heading(&self, heading: &str, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| editor.jump_to_heading(heading, cx));
        }
    }

    pub(super) fn focus_active_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor() {
            editor.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// "Open": shows the note in the active tab (replacing that tab's note), or
    /// in a new tab if none is open. A note that already has a tab just
    /// switches to it, since two editors on one file would overwrite each
    /// other's saves.
    pub(super) fn open_note(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tab_of(&id) {
            return self.activate_tab(ix, window, cx);
        }
        let Some(active) = self.tabs.active_index() else {
            return self.open_note_in_new_tab(id, window, cx);
        };
        // Save the outgoing note ourselves before replacing its editor: an
        // `EditorEvent::Changed` from the last keystroke could still be queued, and
        // without this, switching notes right after typing would drop it.
        self.persist_tab(active, cx);
        let Some(tab) = self.new_tab(&id, window, cx) else {
            return;
        };
        self.tabs[active] = tab;
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    /// "Open in new tab": always a new tab, unless the note already has one.
    pub(super) fn open_note_in_new_tab(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.tab_of(&id) {
            return self.activate_tab(ix, window, cx);
        }
        let Some(tab) = self.new_tab(&id, window, cx) else {
            return;
        };
        self.tabs.push(tab);
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub(super) fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.activate(ix) {
            return;
        }
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    /// Saves the tab's note, then closes it.
    pub(super) fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.persist_tab(ix, cx);
        let was_active = self.tabs.active_index() == Some(ix);
        self.tabs.remove(ix);
        if was_active {
            self.focus_active_editor(window, cx);
        }
        cx.notify();
    }

    /// Writes the tab's text to the note's file straight from the buffer (no
    /// copy) and refreshes the note's cached links.
    pub(super) fn persist_tab(&mut self, ix: usize, cx: &App) {
        let tab = &mut self.tabs[ix];
        let _ = self
            .vault
            .save_note(&tab.note_id, tab.editor.read(cx).buffer().text());
        tab.dirty = false;
        self.search.mark_stale();
        self.content_generation += 1;
    }

    /// Saves every open note: before vault operations that change note ids, and
    /// at quit. Unlike the autosave this does not rely on `dirty`, which is set
    /// by an event that can still be queued.
    pub(super) fn persist_all_tabs(&mut self, cx: &App) {
        for ix in 0..self.tabs.len() {
            self.persist_tab(ix, cx);
        }
    }

    pub(super) fn note_changed(&mut self, editor: EntityId, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.editor.entity_id() == editor)
        {
            tab.dirty = true;
            let (editor, id) = (tab.editor.clone(), tab.note_id.clone());
            self.autosave(cx);
            self.resolve_links_in(&editor, &id, false, cx);
        }
    }

    /// Marks the links in `editor` (the open note `id`) whose target is
    /// missing. With `all`, re-checks every link, not just the unchecked ones.
    pub(super) fn resolve_links_in(
        &self,
        editor: &Entity<Editor>,
        id: &str,
        all: bool,
        cx: &mut Context<Self>,
    ) {
        let vault = &self.vault;
        editor.update(cx, |editor, cx| {
            editor.resolve_links(all, |kind, dest| link_is_live(vault, id, kind, dest), cx);
        });
    }

    /// After the vault's notes changed: every open note's links may now
    /// resolve or break.
    pub(super) fn refresh_links(&mut self, cx: &mut Context<Self>) {
        self.content_generation += 1;
        let tabs: Vec<(Entity<Editor>, String)> = self
            .tabs
            .iter()
            .map(|tab| (tab.editor.clone(), tab.note_id.clone()))
            .collect();
        for (editor, id) in tabs {
            self.resolve_links_in(&editor, &id, true, cx);
        }
    }

    /// Schedules a save of the dirty tabs, restarting the wait on every call:
    /// saving rescans the whole note for wikilinks and rewrites the file, which
    /// is far too much work to do per keystroke.
    pub(super) fn autosave(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DEBOUNCE).await;
            this.update(cx, |this, cx| this.save_dirty_tabs(cx)).ok();
        });
        self.tabs.set_autosave(Some(task));
    }

    /// Persists the dirty tabs, and re-renders so e.g. the backlinks panel
    /// reflects newly-typed wikilinks. Only dirty ones: a background tab holding
    /// a huge note must not be rewritten for every edit elsewhere.
    pub(super) fn save_dirty_tabs(&mut self, cx: &mut Context<Self>) {
        self.tabs.set_autosave(None);
        for ix in 0..self.tabs.len() {
            if self.tabs[ix].dirty {
                self.persist_tab(ix, cx);
            }
        }
        cx.notify();
    }

    /// After a vault rename moved notes: reloads each tab whose note id `moved`
    /// maps to a new one, from disk (the caller saved first).
    pub(super) fn retarget_tabs(
        &mut self,
        moved: impl Fn(&str) -> Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut reloaded_active = false;
        for ix in (0..self.tabs.len()).rev() {
            let Some(new_id) = moved(&self.tabs[ix].note_id) else {
                continue;
            };
            match self.new_tab(&new_id, window, cx) {
                Some(tab) => {
                    self.tabs[ix] = tab;
                    reloaded_active |= self.tabs.active_index() == Some(ix);
                }
                None => self.tabs.remove(ix),
            }
        }
        if reloaded_active {
            self.focus_active_editor(window, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::active_after_remove;

    #[test]
    fn removing_a_tab_keeps_the_active_index_pointing_at_a_tab() {
        // The active tab goes: its right neighbour takes over, else the left.
        assert_eq!(active_after_remove(Some(1), 1, 2), Some(1));
        assert_eq!(active_after_remove(Some(2), 2, 2), Some(1));
        // A tab before the active one goes: the index shifts down.
        assert_eq!(active_after_remove(Some(2), 0, 2), Some(1));
        // A tab after it goes: nothing changes.
        assert_eq!(active_after_remove(Some(0), 2, 2), Some(0));
        // The last tab goes.
        assert_eq!(active_after_remove(Some(0), 0, 0), None);
        assert_eq!(active_after_remove(None, 0, 1), None);
    }
}

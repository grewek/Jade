//! Search over the vault: the query box, the background search and its results.
//! [`Search`] owns the state; `render_search_panel` is the sidebar tab showing it.

#[allow(unused_imports)]
use super::*;

/// The Search tab's state. Search reads note files, so results are computed
/// once per change (query or note text/set), never per frame: a mutation marks
/// the results stale and the next render of the tab starts a search.
pub(crate) struct Search {
    input: Entity<InputState>,
    query: String,
    results: Vec<(String, String)>,
    stale: bool,
    /// Bumped per started search; a finished one whose generation is no
    /// longer current is dropped.
    generation: u64,
    /// The in-flight background search; dropping it cancels the wait.
    task: Option<Task<()>>,
    _subscription: Subscription,
}

impl Search {
    pub(super) fn new(window: &mut Window, cx: &mut Context<JadeApp>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search notes..."));
        let subscription = cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search.query = input.read(cx).value().to_string();
                this.search.stale = true;
                cx.notify();
            }
        });
        Self {
            input,
            query: String::new(),
            results: Vec::new(),
            stale: true,
            generation: 0,
            task: None,
            _subscription: subscription,
        }
    }

    /// The results no longer match the vault or the query: search again at the
    /// next render of the tab.
    pub(super) fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub(super) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// `(note id, title)` of each match, ordered by title.
    pub(crate) fn results(&self) -> &[(String, String)] {
        &self.results
    }

    /// Starts a background search if anything the results depend on changed
    /// since the last call: searching reads every note file, so it must not
    /// block the UI thread. The previous results stay on screen until the new
    /// ones arrive.
    pub(super) fn refresh(&mut self, vault: &Vault, cx: &mut Context<JadeApp>) {
        if !self.stale {
            return;
        }
        self.stale = false;
        self.generation += 1;
        let generation = self.generation;
        let query = self.query.trim().to_string();
        if query.is_empty() {
            self.task = None;
            self.results.clear();
            return;
        }
        let targets = vault.search_targets();
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut results = cx
                .background_spawn(async move { search_targets(&targets, &query) })
                .await;
            results.sort_by(|a, b| a.1.cmp(&b.1));
            this.update(cx, |this, cx| {
                if this.search.generation == generation {
                    this.search.results = results;
                    this.search.task = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Test seam: types `query` without going through the input box.
    #[cfg(test)]
    pub(super) fn set_query_for_test(&mut self, query: &str) {
        self.query = query.to_string();
    }
}

impl JadeApp {
    /// The "Search" sidebar tab: a live-filtered list of notes matching the
    /// query, by title or content substring (via `Vault::search`).
    pub(super) fn render_search_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.search.refresh(&self.vault, cx);
        let mut panel = div().flex().flex_col().gap_1().child(
            Input::new(self.search.input())
                .id("search-input")
                .debug_name("search-input"),
        );

        let query = self.search.query().trim();
        if query.is_empty() {
            panel = panel.child(
                div()
                    .text_color(ThemeEngine::color(cx, Role::TextMuted))
                    .child("Type to search notes by title or content."),
            );
        } else {
            let results = self.search.results().to_vec();

            panel = panel.child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(format!("Results ({})", results.len())),
            );
            if results.is_empty() {
                panel = panel.child(
                    div()
                        .text_color(ThemeEngine::color(cx, Role::TextMuted))
                        .child("No matching notes."),
                );
            } else {
                let row_hover = ThemeEngine::color(cx, Role::Surface);
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
                            .hover(move |style| style.bg(row_hover))
                            .child(title)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.open_note(target_id.clone(), window, cx);
                            }))
                            .debug_name_with(|| format!("search-result-{id}").into()),
                    );
                }
            }
        }
        panel.into_any_element()
    }
}

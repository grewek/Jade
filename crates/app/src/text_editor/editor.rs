use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::*;

mod column_edit;
pub(super) use column_edit::ColumnMenu;
mod table_edit;

use super::EditorBuffer;
use super::columns::handles::ColumnDrag;
use super::doc::{DocumentCache, TextEdit};
use super::doc_stats::DocStats;
use super::element::{render_editor, render_params};
use super::headings::find_heading_line;
use super::image;
use super::link_table::LinkTable;
use super::preview::callout::callout_index_at;
use super::rows::{Change, Inputs, Invalidate, RowCache};
use super::table::handles::{Handle, TableDrag};
use jade_core::link_scan::{LinkKind, scan_line};

/// The `TextEdit` for a plain insertion of `inserted_len` bytes replacing
/// `old_range` (the selection, or the collapsed cursor if none) — call sites
/// capture `old_range` via `buffer.selection()`/`cursor()` *before* calling
/// `EditorBuffer::insert`, since nothing after that call can recover it.
fn edit_for_insertion(old_range: Range<usize>, inserted_len: usize) -> TextEdit {
    TextEdit {
        start_byte: old_range.start,
        old_end_byte: old_range.end,
        new_end_byte: old_range.start + inserted_len,
    }
}

/// The `TextEdit` for deleting `old_selection`, or `None` if there wasn't
/// one — the no-selection case (delete one char before/after the cursor)
/// has no range to report here at all, since without a selection there's
/// nothing to read before the mutating call; callers derive that case
/// themselves from before/after `cursor()`/`text().len()` reads instead
/// (see `Editor::handle_key_down`'s own `"backspace"`/`"delete"` arms).
fn edit_for_deletion(old_selection: Option<Range<usize>>) -> Option<TextEdit> {
    old_selection.map(|range| TextEdit {
        start_byte: range.start,
        old_end_byte: range.end,
        new_end_byte: range.start,
    })
}

/// Emitted whenever a buffer-mutating operation changes the text (typing,
/// IME commit, paste, cut, undo/redo) — not for cursor-only movement or
/// selection changes. `JadeApp` subscribes to this the same way it would to
/// `InputEvent::Change` on a `gpui-component` input, to autosave the open
/// note and refresh anything (e.g. the backlinks panel) that depends on its
/// live content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    Changed,
    /// A link was clicked to follow it; the editor has not moved its cursor.
    OpenLink {
        kind: LinkKind,
        dest: String,
        /// A wikilink's `#heading`; other links carry theirs inside `dest`.
        heading: Option<String>,
    },
}

/// The entity backing one editor view — owns the buffer and the focus handle
/// its element paints a cursor against and attaches mouse/keyboard handlers to.
pub struct Editor {
    pub(super) buffer: EditorBuffer,
    pub(super) focus_handle: FocusHandle,
    /// Whether the left mouse button is currently held down after a press
    /// that started inside this editor, so a subsequent `MouseMoveEvent`
    /// should extend the selection rather than being ignored. Cleared on
    /// `MouseUpEvent` regardless of where the button is released.
    pub(super) is_dragging: bool,
    /// The byte range of text currently under IME composition (e.g. pinyin
    /// not yet confirmed into hanzi), if any — see `input_handler.rs`. Not
    /// yet rendered with the underline real IME composition conventionally
    /// gets; a Phase 1 cosmetic gap, not a functional one.
    pub(super) marked_range: Option<std::ops::Range<usize>>,
    /// Tracks the scroll position of *this editor's* note-editor-pane
    /// container in `app.rs` — owned here (not by `JadeApp`) so it resets
    /// naturally per opened note, same lifetime as everything else about
    /// this `Editor`. `app.rs` calls `.track_scroll(editor.scroll_handle())`
    /// on that container; `element.rs`'s `render_editor` reads/writes it
    /// every paint to keep the cursor's line scrolled into view (see
    /// `autoscroll_to_cursor`) — two different modules sharing one handle,
    /// which is exactly what `ScrollHandle`'s `Rc<RefCell<_>>` is for.
    pub(super) scroll_handle: ScrollHandle,
    /// The buffer cursor offset `autoscroll_to_cursor` last scrolled the
    /// view to, or `None` before the first paint. Needed because
    /// `element.rs`'s `render_editor` rebuilds its whole element tree (new
    /// `canvas` closures included) on *every* actual screen repaint, not
    /// just ones where the cursor moved — a plain mouse-wheel scroll alone
    /// triggers one too (e.g. via `window.refresh()`). Without this,
    /// `autoscroll_to_cursor` ran unconditionally on every such repaint and
    /// snapped the view straight back to the (unchanged) cursor's line the
    /// moment it scrolled out of view, making it impossible to scroll away
    /// from the cursor at all. An `Rc<Cell<_>>` (not a plain field) for the
    /// same reason `scroll_handle` above is an `Rc`-backed handle rather
    /// than a plain `usize`: `render_editor` clones it out of `&Editor`
    /// synchronously (safe, since that's a direct field read) and moves the
    /// clone into the 'static `canvas` paint closure, which mutates it
    /// directly — no need to read/update the entity itself from inside
    /// paint, where that isn't guaranteed safe.
    pub(super) last_autoscrolled_cursor: Rc<Cell<Option<usize>>>,
    /// What the last paint laid out, for keys that need where things are on
    /// screen (Up and Down by painted row).
    pub(super) last_view: Option<Rc<super::element::FrameView>>,
    /// The x a run of Up/Down presses aims for, with the caret it was kept for.
    pub(super) visual_goal: Option<(usize, gpui::Pixels)>,
    /// Persisted classification results for the buffer's current text —
    /// see `document_cache`'s own module doc. `Rc<RefCell<_>>` for the same
    /// reason `scroll_handle`/`last_autoscrolled_cursor` are: `render_editor`
    /// clones it out of `&Editor` synchronously, then reads it (so far —
    /// nothing inside the paint closure mutates it yet) from inside the
    /// `'static` canvas paint closure without touching the entity.
    pub(super) document_cache: Rc<RefCell<DocumentCache>>,
    /// Persisted per-line row-height estimates for the buffer's current
    /// text — see `element::RowCache`'s own module doc. Starts empty
    /// (`RowCache::empty()`): unlike `document_cache` above, building
    /// this needs a `Font`/`Window` to measure a monospace character's
    /// advance width, neither of which `Editor::new` has — it's populated
    /// lazily by `element::analyze_line_layout`'s fingerprint check on the
    /// first real paint instead, the same "eager cost deferred to first
    /// use" shape `available_width`'s own `Pixels::MAX`-before-first-layout
    /// fallback already has. Same `Rc<RefCell<_>>` sharing reasoning as
    /// `document_cache`.
    pub(super) row_height_cache: Rc<RefCell<RowCache>>,
    /// Word and character counts for the status bar, patched per edit.
    doc_stats: DocStats,
    /// Where the note's links are, patched per edit.
    pub(super) link_table: LinkTable,
    /// The mouse is over a link a click would follow: paint shows a pointer.
    pub(super) hovering_link: bool,
    /// The link under the pointer (line, byte span in the line), for hover underlines.
    pub(super) hover_link: Option<(usize, std::ops::Range<usize>)>,
    /// The table Ctrl+/ asked to show raw; it stays raw while the caret is in it.
    raw_pinned: Option<usize>,
    /// The columns region Ctrl+/ asked to show as source, while the caret is in it.
    raw_region_pinned: Option<usize>,
    /// The controls shown on a table's edge for the pointer: `(table, controls)`.
    pub(super) table_hover: Option<(usize, Vec<Handle>)>,
    /// A column or row grip being dragged.
    pub(super) table_drag: Option<TableDrag>,
    /// The columns region whose controls are shown for the pointer.
    pub(super) column_hover: Option<usize>,
    /// Where the last frame put each aside's floats, by region's first line
    /// (for placing a region's grips beside them).
    pub(super) float_spots: Rc<RefCell<Vec<(usize, Vec<super::element::region::FloatSpot>)>>>,
    /// A divider being dragged.
    pub(super) column_drag: Option<ColumnDrag>,
    /// The vault root and this note's id, for finding image embeds' files.
    /// `None` (tests, a note not in a vault) leaves image blocks at their
    /// placeholder height.
    image_context: Option<Arc<(PathBuf, String)>>,
}

impl Editor {
    pub fn new(text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        let buffer = EditorBuffer::new(text);
        // Eager, one-time cost at open — same decision already made for
        // Part 1's viewport-rendering fix (see CLAUDE.md), not a new one
        // for this cache specifically.
        let document_cache = Rc::new(RefCell::new(DocumentCache::new(buffer.text())));
        let doc_stats = DocStats::new(buffer.text());
        let link_table = LinkTable::new(buffer.text(), &document_cache.borrow());
        Self {
            buffer,
            focus_handle: cx.focus_handle(),
            is_dragging: false,
            marked_range: None,
            scroll_handle: ScrollHandle::new(),
            last_autoscrolled_cursor: Rc::new(Cell::new(None)),
            last_view: None,
            visual_goal: None,
            document_cache,
            row_height_cache: Rc::new(RefCell::new(RowCache::empty())),
            doc_stats,
            link_table,
            hovering_link: false,
            hover_link: None,
            raw_pinned: None,
            raw_region_pinned: None,
            table_hover: None,
            table_drag: None,
            column_hover: None,
            float_spots: Rc::new(RefCell::new(Vec::new())),
            column_drag: None,
            image_context: None,
        }
    }

    /// Flips the task checkbox whose `[ ] `/`[x] ` text is `checkbox`. One
    /// undoable byte swap; the cursor, which the swap would move, stays where it was.
    pub(super) fn toggle_checkbox(
        &mut self,
        checkbox: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mark = checkbox.start + 1;
        let flipped = match self.buffer.text().as_bytes().get(mark) {
            Some(b' ') => "x",
            Some(b'x' | b'X') => " ",
            _ => return,
        };
        let selection = self.buffer.selection();
        let cursor = self.buffer.cursor();
        self.buffer.replace_range_with(mark..mark + 1, flipped);
        match selection {
            Some(range) if cursor == range.end => {
                self.buffer.set_cursor(range.start, false);
                self.buffer.set_cursor(range.end, true);
            }
            Some(range) => {
                self.buffer.set_cursor(range.end, false);
                self.buffer.set_cursor(range.start, true);
            }
            None => self.buffer.set_cursor(cursor, false),
        }
        let edit = TextEdit {
            start_byte: mark,
            old_end_byte: mark + 1,
            new_end_byte: mark + 1,
        };
        self.record_edit(edit, window, cx);
    }

    /// Folds callout `index` if it is open, else opens it. Only the view: the
    /// text is untouched. Every row count is rebuilt on the next paint, since
    /// the body's lines go from 0 rows to their own or back.
    pub(super) fn toggle_fold(&mut self, index: usize, cx: &mut Context<Self>) {
        let folded = self
            .document_cache
            .borrow()
            .callouts()
            .get(index)
            .map(|c| c.folded);
        if let Some(folded) = folded {
            if !folded {
                // A cursor left in the body would just open it again.
                let doc = self.document_cache.borrow();
                let header = doc.callouts()[index].start_line;
                let line = doc.line_at(self.buffer.cursor());
                if doc.callouts()[index].end_line > line && line > header {
                    let start = doc.line_start_byte(header);
                    drop(doc);
                    self.buffer.set_cursor(start, false);
                }
            }
            let lines = {
                let mut doc = self.document_cache.borrow_mut();
                doc.set_folded(index, !folded);
                let callout = &doc.callouts()[index];
                callout.start_line..callout.end_line
            };
            self.row_height_cache
                .borrow_mut()
                .invalidate(Invalidate::Lines(lines));
            // The caret stays put, wherever it is: folding must not scroll back
            // to it (a first click on a fold marker in a note not yet focused
            // would send the view to a caret at the top).
            if self.last_autoscrolled_cursor.get().is_none() {
                self.last_autoscrolled_cursor
                    .set(Some(self.buffer.cursor()));
            }
            cx.notify();
        }
    }

    /// A cursor inside a folded callout's body (an arrow key, a search jump)
    /// opens it: the cursor must never sit on a line with no rows.
    fn unfold_at_cursor(&mut self) {
        let mut doc = self.document_cache.borrow_mut();
        let line = doc.line_at(self.buffer.cursor());
        if let Some(index) = callout_index_at(doc.callouts(), line)
            && doc.callouts()[index].hides(line)
        {
            doc.set_folded(index, false);
            self.row_height_cache
                .borrow_mut()
                .invalidate(Invalidate::Width);
        }
    }

    /// Says where this note lives so its image embeds can be found, and reads
    /// the sizes of the ones already in the text.
    pub fn set_image_context(&mut self, root: PathBuf, note_id: String, cx: &mut Context<Self>) {
        self.image_context = Some(Arc::new((root, note_id)));
        let lines = self.document_cache.borrow().line_count();
        self.request_image_sizes(0..lines, cx);
    }

    /// Reads, off the main thread, the headers of image embeds in `lines`
    /// whose size is not known yet; when they arrive every row count is
    /// rebuilt, since each changes a block's height.
    fn request_image_sizes(&self, lines: Range<usize>, cx: &mut Context<Self>) {
        let Some(context) = self.image_context.clone() else {
            return;
        };
        let targets = {
            let mut doc = self.document_cache.borrow_mut();
            let targets = image::unknown_targets(self.buffer.text(), &doc, lines);
            // Marked as asked for, so a second edit of the line does not ask again.
            for target in &targets {
                doc.view.images_mut().insert(target, None);
            }
            targets
        };
        if targets.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let sizes = cx
                .background_spawn(async move {
                    let (root, note_id) = &*context;
                    let read = |target: &String| image::read_info(root, note_id, target);
                    targets
                        .iter()
                        .map(|t| (t.clone(), read(t)))
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |this, cx| this.apply_image_sizes(&sizes, cx))
                .ok();
        })
        .detach();
    }

    fn apply_image_sizes(
        &mut self,
        sizes: &[(String, Option<image::Info>)],
        cx: &mut Context<Self>,
    ) {
        let mut doc = self.document_cache.borrow_mut();
        for (target, size) in sizes {
            doc.view.images_mut().insert(target, size.clone());
        }
        self.row_height_cache
            .borrow_mut()
            .invalidate(Invalidate::Width);
        cx.notify();
    }

    /// The link under raw byte `offset`, as the event that follows it. The
    /// destination is re-parsed out of the line, since the table stores spans only.
    pub(super) fn link_event_at(&self, offset: usize) -> Option<EditorEvent> {
        let doc = self.document_cache.borrow();
        let line = doc.line_at(offset);
        let start = doc.line_start_byte(line);
        let stored = *self.link_table.at(line, offset - start)?;
        let text = self.buffer.text();
        let line_text = &text[start..doc.line_end_byte(line)];
        let mut event = None;
        scan_line(line_text, |link| {
            if link.span.start == stored.start as usize && link.kind == stored.kind {
                event = Some(EditorEvent::OpenLink {
                    kind: link.kind,
                    dest: line_text[link.dest].to_string(),
                    heading: link.heading.map(|h| line_text[h].to_string()),
                });
            }
        });
        event
    }

    /// Whether the pointer is over a link a click would follow.
    #[cfg(test)]
    pub fn is_hovering_link(&self) -> bool {
        self.hovering_link
    }

    /// The link under byte `offset`: its line and byte span within the line.
    pub(super) fn link_span_at(&self, offset: usize) -> Option<(usize, std::ops::Range<usize>)> {
        let doc = self.document_cache.borrow();
        let line = doc.line_at(offset);
        let column = offset - doc.line_start_byte(line);
        let link = self.link_table.at(line, column)?;
        Some((line, link.start as usize..link.end as usize))
    }

    /// Whether a click at `offset` would follow a link (see `on_cursor_line`;
    /// `secondary` is Ctrl/Cmd held).
    pub(super) fn follows_link_at(&self, offset: usize, secondary: bool) -> bool {
        let doc = self.document_cache.borrow();
        let line = doc.line_at(offset);
        let column = offset - doc.line_start_byte(line);
        self.link_table.at(line, column).is_some()
            && (secondary || line != doc.line_at(self.buffer.cursor()))
    }

    /// Whether `offset` lies on the line the cursor is on (shown as raw
    /// Markdown, so a click there edits instead of following).
    pub(super) fn on_cursor_line(&self, offset: usize) -> bool {
        let doc = self.document_cache.borrow();
        doc.line_at(offset) == doc.line_at(self.buffer.cursor())
    }

    /// Marks which links point at nothing; see [`LinkTable::resolve`]. With
    /// `all`, forgets earlier answers first (the vault's notes changed).
    pub(crate) fn resolve_links(
        &mut self,
        all: bool,
        is_live: impl FnMut(LinkKind, &str) -> bool,
        cx: &mut Context<Self>,
    ) {
        if all {
            self.link_table.invalidate();
        }
        let any_broken = {
            let doc = self.document_cache.borrow();
            self.link_table.resolve(self.buffer.text(), &doc, is_live)
        };
        // An edit repaints its own line; only a vault change or a newly
        // broken link elsewhere needs asking for one.
        if all || any_broken {
            cx.notify();
        }
    }

    /// Moves the cursor to the start of zero-based `line` (the last line if
    /// there are fewer) and scrolls to it.
    pub fn jump_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let offset = {
            let doc = self.document_cache.borrow();
            doc.line_start_byte(line.min(doc.line_count().saturating_sub(1)))
        };
        self.buffer.set_cursor(offset, false);
        cx.notify();
    }

    /// Moves the cursor to the first heading matching `heading` and scrolls
    /// to it. Returns whether there was one.
    pub fn jump_to_heading(&mut self, heading: &str, cx: &mut Context<Self>) -> bool {
        let line = {
            let doc = self.document_cache.borrow();
            find_heading_line(self.buffer.text(), &doc, heading)
                .map(|line| doc.line_start_byte(line))
        };
        let Some(offset) = line else {
            return false;
        };
        self.buffer.set_cursor(offset, false);
        cx.notify();
        true
    }

    /// How many of the note's links point at nothing (see `link_table`).
    #[cfg_attr(
        not(any(test, feature = "debug-socket")),
        allow(dead_code, reason = "read by the harness's `app` query")
    )]
    pub fn broken_link_count(&self) -> usize {
        self.link_table.broken_count()
    }

    /// How many links the note has (see `link_table`).
    #[cfg_attr(
        not(any(test, feature = "debug-socket")),
        allow(dead_code, reason = "read by the harness's `app` query")
    )]
    pub fn link_count(&self) -> usize {
        self.link_table.len()
    }

    /// Words in the note (see `doc_stats` for what counts as one).
    pub fn word_count(&self) -> usize {
        self.doc_stats.words()
    }

    /// Characters in the note, spaces included, line breaks not.
    pub fn char_count(&self) -> usize {
        self.doc_stats.chars()
    }

    pub fn buffer(&self) -> &EditorBuffer {
        &self.buffer
    }

    /// Visible lines `paint_lines` still had to re-measure after the pre-pass.
    #[cfg(test)]
    pub fn late_row_corrections(&self) -> usize {
        self.row_height_cache.borrow().trace_state().2
    }

    pub fn buffer_mut(&mut self) -> &mut EditorBuffer {
        &mut self.buffer
    }

    /// The scroll handle for `app.rs`'s note-editor-pane container to
    /// `.track_scroll(...)` — see the field's own doc comment.
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll_handle
    }

    /// For call sites (here and in `input_handler.rs`) where the buffer's
    /// text itself just changed, not merely its cursor/selection — emits
    /// `EditorEvent::Changed` in addition to the `cx.notify()` every mutation
    /// needs anyway.
    pub(super) fn notify_changed(&self, cx: &mut Context<Self>) {
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Patches `document_cache` for a precisely-known edit, then patches
    /// `row_height_cache` too — using the touched-line range `document_
    /// cache`'s own patch just returned, and the same render parameters
    /// (`font`/`available_width`/etc.) `render_editor` itself computes every
    /// paint, via `element::render_params` — before emitting
    /// `EditorEvent::Changed`. The call every text-mutating call site in
    /// this file and `input_handler.rs` makes instead of a plain
    /// `notify_changed(cx)`, right after the `buffer` mutation itself.
    pub(super) fn record_edit(
        &mut self,
        edit: TextEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.buffer.text();
        let patch = self.document_cache.borrow_mut().record_edit(text, edit);
        match &patch {
            Some(patch) => {
                self.doc_stats
                    .patch(text, &self.document_cache.borrow(), patch);
                self.link_table
                    .patch(text, &self.document_cache.borrow(), patch);
            }
            None => {
                self.doc_stats.rebuild(text);
                self.link_table.rebuild(text, &self.document_cache.borrow());
            }
        }

        let touched = match &patch {
            Some(patch) => patch.touched.clone(),
            None => 0..self.document_cache.borrow().line_count(),
        };
        self.request_image_sizes(touched, cx);

        let params = render_params(self, window, cx);
        let inp = Inputs {
            text,
            cursor: self.buffer.cursor(),
            focused: self.focus_handle.is_focused(window),
            width: params.available_width,
            font: &params.font,
            font_size: params.font_size,
            text_color: params.text_color,
        };
        let mut rows = self.row_height_cache.borrow_mut();
        match &patch {
            Some(patch) => {
                rows.update(&self.document_cache, Change::Edit(patch), &inp, window);
            }
            // No patch to apply: every count is rebuilt.
            None => {
                rows.invalidate(Invalidate::Width);
                rows.update(&self.document_cache, Change::Layout, &inp, window);
            }
        }
        drop(rows);

        self.notify_changed(cx);
    }

    /// Navigation/editing keys only (arrows, Home/End, Backspace/Delete,
    /// Enter, Ctrl/Cmd+A/C/X/V/Z, Ctrl/Cmd+Shift+Z, Ctrl/Cmd+Left/Right for
    /// word-wise movement/selection) — plain character typing goes through
    /// `EntityInputHandler::replace_text_in_range` instead (`input_handler.rs`,
    /// wired via `window.handle_input` during paint), GPUI's platform text
    /// input path, which also handles real IME composition. An earlier
    /// version of this method had a `keystroke.key_char` fallback for typing
    /// before that existed; removed once it did, rather than kept as a
    /// second path for the same keystrokes to arrive through.
    ///
    /// Copy/cut/paste are handled explicitly here (reading/writing
    /// `cx.read_from_clipboard`/`write_to_clipboard` directly) rather than
    /// relying on `EntityInputHandler::paste`'s default — that path is the
    /// platform's own Services-menu/drag-drop paste plumbing on some
    /// platforms, not guaranteed to fire for a plain Ctrl/Cmd+V keystroke, so
    /// explicit is the more predictable choice here.
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let shift = keystroke.modifiers.shift;
        let command = keystroke.modifiers.control || keystroke.modifiers.platform;

        if command {
            match keystroke.key.as_str() {
                "/" => {
                    if !self.toggle_raw_table(cx) {
                        self.toggle_raw_region(cx);
                    }
                    return;
                }
                "a" => {
                    if !self.select_cell() {
                        self.buffer.select_all();
                    }
                    cx.notify();
                    return;
                }
                "c" => {
                    if let Some(text) = self.buffer.copy() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    return;
                }
                "x" => {
                    if let Some(edit) = edit_for_deletion(self.buffer.selection())
                        && let Some(text) = self.buffer.cut()
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        self.record_edit(edit, window, cx);
                    }
                    return;
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        let old_range = self.buffer.selection().unwrap_or_else(|| {
                            let c = self.buffer.cursor();
                            c..c
                        });
                        if let Some((range, text)) = self.cell_input(old_range.clone(), &text) {
                            self.replace_in_buffer(range, &text, window, cx);
                            return;
                        }
                        let edit = edit_for_insertion(old_range, text.len());
                        self.buffer.insert(&text);
                        self.record_edit(edit, window, cx);
                    }
                    return;
                }
                "z" => {
                    let edit = if shift {
                        self.buffer.redo()
                    } else {
                        self.buffer.undo()
                    };
                    if let Some(edit) = edit {
                        self.record_edit(edit, window, cx);
                    }
                    return;
                }
                "left" => {
                    self.buffer.move_word_left(shift);
                    self.skip_hidden_lines(false, shift);
                    cx.notify();
                    return;
                }
                "right" => {
                    self.buffer.move_word_right(shift);
                    self.skip_hidden_lines(true, shift);
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        if !command && self.table_key(keystroke.key.as_str(), shift, window, cx) {
            return;
        }

        match keystroke.key.as_str() {
            "left" => {
                self.buffer.move_left(shift);
                self.skip_hidden_lines(false, shift);
                cx.notify();
            }
            "right" => {
                self.buffer.move_right(shift);
                self.skip_hidden_lines(true, shift);
                cx.notify();
            }
            "up" => {
                if !super::element::move_by_row(self, false, shift, window) {
                    self.move_vertical(false, shift);
                }
                cx.notify();
            }
            "down" => {
                if !super::element::move_by_row(self, true, shift, window) {
                    self.move_vertical(true, shift);
                }
                cx.notify();
            }
            "home" => {
                self.buffer.move_to_line_start(shift);
                cx.notify();
            }
            "end" => {
                self.buffer.move_to_line_end(shift);
                cx.notify();
            }
            // Deletion with an active selection always has a precise
            // `TextEdit` (the selection itself) available *before* the
            // mutating call. Without one, only `delete_backward`'s own
            // before/after `cursor()` reads (it moves; `delete_forward`
            // doesn't) can recover the single removed char's range — no
            // private `EditorBuffer` helper is exposed for it, and none is
            // needed.
            "backspace" => {
                if self.buffer.selection().is_none() && self.at_region_edge(true) {
                    return;
                }
                if let Some(edit) = edit_for_deletion(self.buffer.selection()) {
                    self.buffer.delete_backward();
                    self.record_edit(edit, window, cx);
                } else {
                    let old_cursor = self.buffer.cursor();
                    self.buffer.delete_backward();
                    let new_cursor = self.buffer.cursor();
                    if new_cursor != old_cursor {
                        self.record_edit(
                            TextEdit {
                                start_byte: new_cursor,
                                old_end_byte: old_cursor,
                                new_end_byte: new_cursor,
                            },
                            window,
                            cx,
                        );
                    }
                }
            }
            // `delete_forward`'s cursor never moves, so the removed range's
            // end has to come from the byte-length delta instead.
            "delete" => {
                if self.buffer.selection().is_none() && self.at_region_edge(false) {
                    return;
                }
                if let Some(edit) = edit_for_deletion(self.buffer.selection()) {
                    self.buffer.delete_forward();
                    self.record_edit(edit, window, cx);
                } else {
                    let old_cursor = self.buffer.cursor();
                    let old_len = self.buffer.text().len();
                    self.buffer.delete_forward();
                    let new_len = self.buffer.text().len();
                    if new_len != old_len {
                        self.record_edit(
                            TextEdit {
                                start_byte: old_cursor,
                                old_end_byte: old_cursor + (old_len - new_len),
                                new_end_byte: old_cursor,
                            },
                            window,
                            cx,
                        );
                    }
                }
            }
            "enter" => {
                let old_range = self.buffer.selection().unwrap_or_else(|| {
                    let c = self.buffer.cursor();
                    c..c
                });
                let edit = edit_for_insertion(old_range, 1);
                self.buffer.insert("\n");
                self.record_edit(edit, window, cx);
            }
            _ => {}
        }
    }

    /// Inserts a literal tab character. Not handled inside `handle_key_down`
    /// like every other key, because a plain `on_key_down` listener can't
    /// win here: `gpui_kit::init`'s global `Root` component already binds
    /// `"tab"` to its own `Tab` action (focus-next) in a `"Root"` key
    /// context that wraps the whole app, and GPUI resolves a keystroke
    /// against bound actions — innermost key-context first — *before*
    /// running any element's raw `key_down_listeners` at all. An
    /// `on_key_down` handler simply never sees "tab"; only binding a
    /// *more specific* action ourselves, in a context nested inside
    /// "Root", can take priority over it — the same mechanism `app.rs`
    /// already uses for `RenameNote`/`DeleteNote` (`"NoteList"` context),
    /// just here needing to actually out-rank a default binding rather
    /// than just adding a new one.
    pub(super) fn handle_insert_tab(
        &mut self,
        _: &InsertTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab_in_table(false, window, cx) {
            return;
        }
        let old_range = self.buffer.selection().unwrap_or_else(|| {
            let c = self.buffer.cursor();
            c..c
        });
        let edit = edit_for_insertion(old_range, 1);
        self.buffer.insert("\t");
        self.record_edit(edit, window, cx);
    }

    /// Shift+Tab: the previous table cell; nothing outside a table.
    pub(super) fn handle_back_tab(
        &mut self,
        _: &BackTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_in_table(true, window, cx);
    }
}

actions!(
    text_editor,
    [
        InsertTab,
        BackTab,
        TableRowAbove,
        TableRowBelow,
        TableColumnLeft,
        TableColumnRight,
        TableDeleteRow,
        TableDeleteColumn,
        TableAlignLeft,
        TableAlignCenter,
        TableAlignRight
    ]
);

/// The editor's key bindings, all in the `TextEditor` context. Tab and
/// Shift+Tab must out-rank the global focus-next binding (see
/// [`Editor::handle_insert_tab`]); the table chords are Alt+Shift so that
/// nothing else uses them, and are given back (propagated) outside a table.
pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some("TextEditor");
    vec![
        KeyBinding::new("tab", InsertTab, context),
        KeyBinding::new("shift-tab", BackTab, context),
        KeyBinding::new("alt-shift-up", TableRowAbove, context),
        KeyBinding::new("alt-shift-down", TableRowBelow, context),
        KeyBinding::new("alt-shift-left", TableColumnLeft, context),
        KeyBinding::new("alt-shift-right", TableColumnRight, context),
        KeyBinding::new("alt-shift-backspace", TableDeleteRow, context),
        KeyBinding::new("alt-shift-delete", TableDeleteColumn, context),
        KeyBinding::new("alt-shift-l", TableAlignLeft, context),
        KeyBinding::new("alt-shift-c", TableAlignCenter, context),
        KeyBinding::new("alt-shift-r", TableAlignRight, context),
    ]
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.unfold_at_cursor();
        self.sync_table_view(window);
        self.sync_column_view(window, cx);
        self.document_cache
            .borrow_mut()
            .view
            .set_justify(super::TextSettings::justify(cx));
        render_editor(self, cx.entity(), window, cx)
    }
}

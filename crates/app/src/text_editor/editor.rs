use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::*;

use super::EditorBuffer;
use super::document_cache::{DocumentCache, TextEdit};
use super::element::{RowHeightCache, mono_char_width, render_editor, render_params};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorEvent {
    Changed,
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
    /// container in `main.rs` — owned here (not by `JadeApp`) so it resets
    /// naturally per opened note, same lifetime as everything else about
    /// this `Editor`. `main.rs` calls `.track_scroll(editor.scroll_handle())`
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
    /// Persisted classification results for the buffer's current text —
    /// see `document_cache`'s own module doc. `Rc<RefCell<_>>` for the same
    /// reason `scroll_handle`/`last_autoscrolled_cursor` are: `render_editor`
    /// clones it out of `&Editor` synchronously, then reads it (so far —
    /// nothing inside the paint closure mutates it yet) from inside the
    /// `'static` canvas paint closure without touching the entity.
    pub(super) document_cache: Rc<RefCell<DocumentCache>>,
    /// Persisted per-line row-height estimates for the buffer's current
    /// text — see `element::RowHeightCache`'s own module doc. Starts empty
    /// (`RowHeightCache::empty()`): unlike `document_cache` above, building
    /// this needs a `Font`/`Window` to measure a monospace character's
    /// advance width, neither of which `Editor::new` has — it's populated
    /// lazily by `element::analyze_line_layout`'s fingerprint check on the
    /// first real paint instead, the same "eager cost deferred to first
    /// use" shape `available_width`'s own `Pixels::MAX`-before-first-layout
    /// fallback already has. Same `Rc<RefCell<_>>` sharing reasoning as
    /// `document_cache`.
    pub(super) row_height_cache: Rc<RefCell<RowHeightCache>>,
}

impl Editor {
    pub fn new(text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        let buffer = EditorBuffer::new(text);
        // Eager, one-time cost at open — same decision already made for
        // Part 1's viewport-rendering fix (see CLAUDE.md), not a new one
        // for this cache specifically.
        let document_cache = Rc::new(RefCell::new(DocumentCache::new(buffer.text())));
        Self {
            buffer,
            focus_handle: cx.focus_handle(),
            is_dragging: false,
            marked_range: None,
            scroll_handle: ScrollHandle::new(),
            last_autoscrolled_cursor: Rc::new(Cell::new(None)),
            document_cache,
            row_height_cache: Rc::new(RefCell::new(RowHeightCache::empty())),
        }
    }

    pub fn buffer(&self) -> &EditorBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut EditorBuffer {
        &mut self.buffer
    }

    /// The scroll handle for `main.rs`'s note-editor-pane container to
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

        let params = render_params(self, window, cx);
        let cursor = self.buffer.cursor();
        let focused = self.focus_handle.is_focused(window);
        let char_width = mono_char_width(&params.font, params.font_size, params.text_color, window);
        let doc = self.document_cache.borrow();
        let mut rows = self.row_height_cache.borrow_mut();
        if let Some(patch) = &patch {
            rows.resize_for(patch, &doc);
        }
        match patch.map(|p| p.touched) {
            Some(range) if !rows.needs_full_rebuild(&doc, params.available_width, char_width) => {
                rows.patch_lines(
                    text,
                    &doc,
                    range,
                    cursor,
                    focused,
                    char_width,
                    &params.font,
                    params.font_size,
                    params.text_color,
                    window,
                );
            }
            _ => {
                rows.rebuild(
                    text,
                    &doc,
                    cursor,
                    focused,
                    params.available_width,
                    char_width,
                    &params.font,
                    params.font_size,
                    params.text_color,
                    window,
                );
            }
        }
        drop(rows);
        drop(doc);

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
                "a" => {
                    self.buffer.select_all();
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
                    cx.notify();
                    return;
                }
                "right" => {
                    self.buffer.move_word_right(shift);
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match keystroke.key.as_str() {
            "left" => {
                self.buffer.move_left(shift);
                cx.notify();
            }
            "right" => {
                self.buffer.move_right(shift);
                cx.notify();
            }
            "up" => {
                self.buffer.move_up(shift);
                cx.notify();
            }
            "down" => {
                self.buffer.move_down(shift);
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
    /// "Root", can take priority over it — the same mechanism `main.rs`
    /// already uses for `RenameNote`/`DeleteNote` (`"NoteList"` context),
    /// just here needing to actually out-rank a default binding rather
    /// than just adding a new one.
    pub(super) fn handle_insert_tab(
        &mut self,
        _: &InsertTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_range = self.buffer.selection().unwrap_or_else(|| {
            let c = self.buffer.cursor();
            c..c
        });
        let edit = edit_for_insertion(old_range, 1);
        self.buffer.insert("\t");
        self.record_edit(edit, window, cx);
    }
}

actions!(text_editor, [InsertTab]);

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_editor(self, cx.entity(), window, cx)
    }
}

use gpui_kit::*;

use super::element::render_editor;
use super::EditorBuffer;

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
}

impl Editor {
    pub fn new(text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        Self {
            buffer: EditorBuffer::new(text),
            focus_handle: cx.focus_handle(),
            is_dragging: false,
            marked_range: None,
        }
    }

    pub fn buffer(&self) -> &EditorBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut EditorBuffer {
        &mut self.buffer
    }

    /// Navigation/editing keys only (arrows, Home/End, Backspace/Delete,
    /// Enter, Ctrl/Cmd+A/C/X/V) — plain character typing goes through
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
    pub(super) fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
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
                    if let Some(text) = self.buffer.cut() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        cx.notify();
                    }
                    return;
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.buffer.insert(&text);
                        cx.notify();
                    }
                    return;
                }
                _ => {}
            }
        }

        match keystroke.key.as_str() {
            "left" => self.buffer.move_left(shift),
            "right" => self.buffer.move_right(shift),
            "up" => self.buffer.move_up(shift),
            "down" => self.buffer.move_down(shift),
            "home" => self.buffer.move_to_line_start(shift),
            "end" => self.buffer.move_to_line_end(shift),
            "backspace" => self.buffer.delete_backward(),
            "delete" => self.buffer.delete_forward(),
            "enter" => self.buffer.insert("\n"),
            _ => return,
        }
        cx.notify();
    }
}

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

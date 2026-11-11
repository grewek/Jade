use gpui_kit::*;

use super::element::render_editor;
use super::EditorBuffer;

/// The entity backing one editor view — owns the buffer and the focus handle
/// its element paints a cursor against and attaches mouse/keyboard handlers to.
pub struct Editor {
    buffer: EditorBuffer,
    pub(super) focus_handle: FocusHandle,
    /// Whether the left mouse button is currently held down after a press
    /// that started inside this editor, so a subsequent `MouseMoveEvent`
    /// should extend the selection rather than being ignored. Cleared on
    /// `MouseUpEvent` regardless of where the button is released.
    pub(super) is_dragging: bool,
}

impl Editor {
    pub fn new(text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        Self { buffer: EditorBuffer::new(text), focus_handle: cx.focus_handle(), is_dragging: false }
    }

    pub fn buffer(&self) -> &EditorBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut EditorBuffer {
        &mut self.buffer
    }

    /// Navigation/editing keys (arrows, Home/End, Backspace/Delete, Enter,
    /// Ctrl/Cmd+A), plus — for keys that aren't one of those — plain
    /// character typing via `keystroke.key_char` when no significant
    /// modifier is held.
    ///
    /// This is a Phase 1 simplification, not the final input path: real text
    /// input (proper IME composition, non-Latin scripts) is a later Phase 1
    /// step (`EntityInputHandler`, wired through `window.handle_input`) that
    /// supersedes the plain-typing half of this once it lands — the
    /// navigation/editing-key half stays as-is either way, since those
    /// aren't something IME composition is involved in.
    pub(super) fn handle_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let shift = keystroke.modifiers.shift;
        let no_significant_modifier =
            !keystroke.modifiers.control && !keystroke.modifiers.platform && !keystroke.modifiers.function;

        if (keystroke.modifiers.control || keystroke.modifiers.platform) && keystroke.key == "a" {
            self.buffer.select_all();
            cx.notify();
            return;
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
            _ => {
                let Some(text) = (no_significant_modifier).then_some(keystroke.key_char.as_deref()).flatten()
                else {
                    return;
                };
                self.buffer.insert(text);
            }
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

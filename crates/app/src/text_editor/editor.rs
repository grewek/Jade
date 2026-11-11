use gpui_kit::*;

use super::element::render_editor;
use super::EditorBuffer;

/// The entity backing one editor view — owns the buffer and the focus handle
/// its element paints a cursor against and will (in a later Phase 1 step)
/// attach mouse/keyboard handlers to.
pub struct Editor {
    buffer: EditorBuffer,
    pub(super) focus_handle: FocusHandle,
}

impl Editor {
    pub fn new(text: impl Into<String>, cx: &mut Context<Self>) -> Self {
        Self { buffer: EditorBuffer::new(text), focus_handle: cx.focus_handle() }
    }

    pub fn buffer(&self) -> &EditorBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut EditorBuffer {
        &mut self.buffer
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_editor(self, window, cx)
    }
}

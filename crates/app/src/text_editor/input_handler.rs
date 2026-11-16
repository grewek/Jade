use std::ops::Range;

use gpui_kit::*;

use super::document_cache::TextEdit;
use super::element::{
    analyze_line_layout, callout_at_line, indent_px, is_active_line, shape_display_line,
    table_line_kind, visible_line_range,
};
use super::{Editor, EditorBuffer};

/// The byte offset in `text` after `utf16_offset` UTF-16 code units.
///
/// `EntityInputHandler` speaks UTF-16 offsets (the OS text-input APIs are
/// UTF-16-native), while the buffer is UTF-8.
pub(super) fn utf16_to_utf8(text: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0;
    for (byte_index, ch) in text.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_index;
        }
        utf16_count += ch.len_utf16();
    }
    text.len()
}

/// The UTF-16 offset of `byte_offset` into `text` — the inverse of
/// [`utf16_to_utf8`].
pub(super) fn utf8_to_utf16(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().map(char::len_utf16).sum()
}

/// The byte range a replace should cover: the explicit `range_utf16`, else
/// the marked range, else the selection/cursor.
///
/// Borrows the text instead of cloning it: `range_utf16` is `None` for every
/// plain keystroke, and a clone cost ~1ms per key on the 320K-line fixture.
fn resolve_replace_range(
    buffer: &EditorBuffer,
    marked_range: &Option<Range<usize>>,
    range_utf16: Option<Range<usize>>,
) -> Range<usize> {
    match range_utf16 {
        Some(r) => {
            let text = buffer.text();
            utf16_to_utf8(text, r.start)..utf16_to_utf8(text, r.end)
        }
        None => marked_range.clone().unwrap_or_else(|| {
            let cursor = buffer.cursor();
            buffer.selection().unwrap_or(cursor..cursor)
        }),
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.buffer.text();
        let start = utf16_to_utf8(text, range_utf16.start).min(text.len());
        let end = utf16_to_utf8(text, range_utf16.end).min(text.len());
        let (start, end) = (start.min(end), start.max(end));
        *adjusted_range = Some(utf8_to_utf16(text, start)..utf8_to_utf16(text, end));
        Some(text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.buffer.text();
        let cursor = self.buffer.cursor();
        let selection = self.buffer.selection().unwrap_or(cursor..cursor);
        Some(UTF16Selection {
            range: utf8_to_utf16(text, selection.start)..utf8_to_utf16(text, selection.end),
            // `EditorBuffer::selection()` is normalized, so the caret side
            // is unknown; only rare IMEs care.
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `None` also means "commit an IME composition" (e.g. a dead-key
        // `^`), which must replace the marked text; ignoring it duplicated
        // the preview character.
        let replace_range = resolve_replace_range(&self.buffer, &self.marked_range, range_utf16);
        let edit = TextEdit {
            start_byte: replace_range.start,
            old_end_byte: replace_range.end,
            new_end_byte: replace_range.start + text.len(),
        };
        self.buffer.replace_range_with(replace_range, text);
        self.marked_range = None;
        self.record_edit(edit, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let replace_range = resolve_replace_range(&self.buffer, &self.marked_range, range_utf16);
        let edit = TextEdit {
            start_byte: replace_range.start,
            old_end_byte: replace_range.end,
            new_end_byte: replace_range.start + new_text.len(),
        };
        let marked_start = replace_range.start;
        self.buffer.replace_range_with(replace_range, new_text);
        let marked_end = marked_start + new_text.len();
        self.marked_range = Some(marked_start..marked_end);

        match new_selected_range {
            // `sel` is UTF-16-relative to the start of `new_text`, not the document.
            Some(sel) => {
                let start = marked_start + utf16_to_utf8(new_text, sel.start);
                let end = marked_start + utf16_to_utf8(new_text, sel.end);
                self.buffer.set_cursor(start, false);
                self.buffer.set_cursor(end, true);
            }
            None => self.buffer.set_cursor(marked_end, false),
        }
        self.record_edit(edit, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let text = self.buffer.text();
        let byte_offset = utf16_to_utf8(text, range_utf16.start);
        let cursor = self.buffer.cursor();
        let line_height = window.line_height();
        let text_style = window.text_style();
        // Must match `render_editor`'s mono-family override, or the IME
        // candidate window lands slightly off.
        let mut font = text_style.font();
        font.family = gpui_kit::component::Theme::global(cx)
            .mono_font_family
            .clone();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let text_color = text_style.color;
        let focused = self.focus_handle.is_focused(window);

        // Viewport-bounded like `paint_lines`: the IME target is near the
        // cursor, which is already scrolled into view.
        let layout = analyze_line_layout(
            text,
            &self.document_cache,
            &self.row_height_cache,
            cursor,
            focused,
            element_bounds.size.width,
            line_height,
            &font,
            font_size,
            text_color,
            window,
        );
        // Reuses the incrementally maintained classification cache instead
        // of reclassifying the document.
        let cache_ref = self.document_cache.borrow();
        let line_roles = cache_ref.line_roles();
        let block_contexts = cache_ref.block_contexts();
        let tables = cache_ref.tables();
        let callouts = cache_ref.callouts();
        let viewport = self.scroll_handle.bounds();
        let range = visible_line_range(&layout, element_bounds, viewport);
        if range.is_empty() {
            return None;
        }

        let mut y_offset = layout.cumulative_at(range.start);
        for i in range {
            let line_range = layout.line_range(i);
            let line_text = &text[line_range.clone()];
            let active = is_active_line(&line_range, cursor, focused);
            let table_kind = table_line_kind(tables, i);
            let table_active = layout.is_table_active(i, cursor, focused);
            let callout = callout_at_line(callouts, i);
            let callout_is_header = callout.is_some_and(|c| c.is_header(i));
            let results = shape_display_line(
                line_text,
                line_roles[i],
                block_contexts[i],
                table_kind,
                table_active,
                callout,
                callout_is_header,
                active,
                &font,
                font_size,
                text_color,
                element_bounds.size.width,
                window,
            );
            // More than one entry only for a table row whose cells wrapped.
            for (wrapped, display_line) in &results {
                let height = wrapped.size(line_height).height;
                if byte_offset <= line_range.end {
                    let display_offset =
                        display_line.display_offset_for_raw(byte_offset - line_range.start);
                    let pos = wrapped
                        .position_for_index(display_offset, line_height)
                        .unwrap_or(point(px(0.), px(0.)));
                    let x = pos.x + indent_px(display_line.indent.level());
                    let origin = element_bounds.origin + point(x, y_offset + pos.y);
                    return Some(Bounds {
                        origin,
                        size: size(px(1.), line_height),
                    });
                }
                y_offset += height;
            }
        }
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // Needs the element's screen bounds, which this method isn't given.
        // Only affects OS affordances like "Look Up", not typing or IME.
        None
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.buffer.text();
        let start = utf16_to_utf8(text, range_utf16.start);
        let end = utf16_to_utf8(text, range_utf16.end);
        self.buffer.set_cursor(start, false);
        self.buffer.set_cursor(end, true);
        cx.notify();
    }
}

// Not `use super::*`: it would pull in GPUI's `test` macro, which shadows
// `#[test]` and recurses infinitely (same gotcha as in `app.rs`).
#[cfg(test)]
mod tests {
    use super::{utf8_to_utf16, utf16_to_utf8};

    #[test]
    fn utf16_offsets_round_trip_through_ascii() {
        let text = "hello world";
        for byte_offset in [0, 1, 5, 11] {
            let utf16 = utf8_to_utf16(text, byte_offset);
            assert_eq!(utf16, byte_offset, "ASCII is 1 UTF-16 unit per byte");
            assert_eq!(utf16_to_utf8(text, utf16), byte_offset);
        }
    }

    #[test]
    fn utf16_offsets_account_for_multi_byte_and_surrogate_pair_chars() {
        // é is 2 UTF-8 bytes but 1 UTF-16 unit; 😀 is 4 UTF-8 bytes but a
        // UTF-16 *surrogate pair* (2 units) — the case plain byte-counting
        // would get wrong.
        let text = "a\u{00e9}\u{1f600}b"; // "a", "é", "😀", "b"
        // byte offsets: a=0..1, é=1..3, 😀=3..7, b=7..8
        assert_eq!(utf8_to_utf16(text, 0), 0);
        assert_eq!(utf8_to_utf16(text, 1), 1); // after "a": 1 UTF-16 unit
        assert_eq!(utf8_to_utf16(text, 3), 2); // after "é": +1 UTF-16 unit
        assert_eq!(utf8_to_utf16(text, 7), 4); // after 😀: +2 UTF-16 units (surrogate pair)
        assert_eq!(utf8_to_utf16(text, 8), 5); // after "b": +1

        assert_eq!(utf16_to_utf8(text, 0), 0);
        assert_eq!(utf16_to_utf8(text, 1), 1);
        assert_eq!(utf16_to_utf8(text, 2), 3);
        assert_eq!(utf16_to_utf8(text, 4), 7);
        assert_eq!(utf16_to_utf8(text, 5), 8);
    }
}

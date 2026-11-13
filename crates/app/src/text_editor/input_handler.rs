use std::ops::Range;

use gpui_kit::*;

use super::document_cache::TextEdit;
use super::element::{
    analyze_line_layout, callout_at_line, indent_px, is_active_line, is_table_active, shape_display_line,
    table_line_kind, visible_line_range,
};
use super::{Editor, EditorBuffer};

/// The byte offset in `text` after `utf16_offset` UTF-16 code units.
///
/// GPUI's platform text input protocol (`EntityInputHandler`) speaks UTF-16
/// offsets, since that's the common denominator for the underlying OS APIs
/// (both macOS's and Windows' text input services are UTF-16-native)
/// regardless of this buffer being UTF-8 internally — every method on this
/// trait needs a conversion at the boundary.
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

/// The byte range `replace_text_in_range`/`replace_and_mark_text_in_range`
/// should replace: the explicit `range_utf16` if given, else whatever's
/// still marked, else the current selection/cursor — see each caller's own
/// doc comment for why that fallback order matters. Deliberately borrows
/// `buffer.text()` rather than cloning it (an earlier version of both
/// callers did `buffer.text().to_string()` unconditionally, even though
/// `range_utf16` is `None` for every plain keystroke — not just real IME
/// composition — making that an O(document) clone paid on every single
/// character typed in a large note, confirmed by direct measurement to
/// cost ~1ms on the 320K-line stress fixture for no reason the `None` case
/// ever needed it for) — the borrow only has to survive long enough to
/// resolve `r`'s UTF-16 offsets, well before either caller's own first
/// mutating `buffer` call.
fn resolve_replace_range(buffer: &EditorBuffer, marked_range: &Option<Range<usize>>, range_utf16: Option<Range<usize>>) -> Range<usize> {
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
            // Not tracked — `EditorBuffer::selection()` only exposes the
            // normalized (start <= end) range, not which side the actual
            // cursor is on. Phase 1 simplification; affects only the rare
            // IME/text-service that cares which end the caret is at.
            reversed: false,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
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
        // `range_utf16` is `None` for, among other cases, committing an IME
        // composition (e.g. a dead-key preview like `^` or `` ` ``) — which
        // must replace whatever's still marked, not insert next to it. Mirror
        // `replace_and_mark_text_in_range`'s own fallback chain (explicit
        // range, else the marked range, else the current selection/cursor):
        // using only the current selection/cursor here left the marked
        // preview character in the buffer and inserted the committed text
        // right after it, duplicating the symbol once the composition ended
        // (e.g. on the next keystroke, like pressing space).
        let replace_range = resolve_replace_range(&self.buffer, &self.marked_range, range_utf16);
        let edit = TextEdit {
            start_byte: replace_range.start,
            old_end_byte: replace_range.end,
            new_end_byte: replace_range.start + text.len(),
        };
        self.buffer.set_cursor(replace_range.start, false);
        self.buffer.set_cursor(replace_range.end, true);
        self.buffer.insert(text);
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
        self.buffer.set_cursor(replace_range.start, false);
        self.buffer.set_cursor(replace_range.end, true);
        self.buffer.insert(new_text);

        let marked_start = replace_range.start;
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
        let text = self.buffer.text().to_string();
        let byte_offset = utf16_to_utf8(&text, range_utf16.start);
        let cursor = self.buffer.cursor();
        let line_height = window.line_height();
        let text_style = window.text_style();
        // Same mono-family override `element.rs`'s `render_editor` applies —
        // bold/italic Live Preview runs need the bundled mono family's real
        // Bold/Italic faces, and this method must shape with the same font
        // that painting did or the IME candidate window lands slightly off.
        let mut font = text_style.font();
        font.family = gpui_kit::component::Theme::global(cx).mono_font_family.clone();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let text_color = text_style.color;
        let focused = self.focus_handle.is_focused(window);

        // Same viewport-bounded approach `element.rs`'s `paint_lines`/
        // `offset_for_position` use — IME composition's target byte offset
        // is normally at/near the cursor, which (per `autoscroll_to_cursor`)
        // should already be inside the visible viewport, so windowing here
        // avoids the same O(document) real-shaping cost those functions
        // used to pay on every call, not just on every paint. Also reuses
        // `row_height_cache` (patched incrementally the same way `document_
        // cache` is — see `Editor::record_edit`) rather than re-deriving
        // every line's row count here too.
        let layout = analyze_line_layout(
            &text, &self.document_cache, &self.row_height_cache, cursor, focused, element_bounds.size.width,
            line_height, &font, font_size, text_color, window,
        );
        // Reads the same persisted classification cache every edit already
        // updated via `record_edit`/`invalidate_all` — this used to
        // independently rerun all four classify_* passes on every call, not
        // just once per paint like `render_editor`; see `document_cache`'s
        // own module doc.
        let cache_ref = self.document_cache.borrow();
        let line_roles = cache_ref.line_roles();
        let block_contexts = cache_ref.block_contexts();
        let table_for_line = cache_ref.table_for_line();
        let tables = cache_ref.tables();
        let callout_for_line = cache_ref.callout_for_line();
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
            let table_kind = table_line_kind(tables, table_for_line, i);
            let table_active = is_table_active(tables, table_for_line, i, cursor, focused);
            let callout = callout_at_line(callouts, callout_for_line, i);
            let callout_is_header = callout.is_some_and(|c| c.is_header(i));
            let results = shape_display_line(
                line_text, line_roles[i], block_contexts[i], table_kind, table_active, callout, callout_is_header,
                active, &font, font_size, text_color, element_bounds.size.width, window,
            );
            // Usually one entry; more than one only for a table row whose
            // cells had to wrap — same "which sub-row is this byte
            // actually on" search, one level deeper.
            for (wrapped, display_line) in &results {
                let height = wrapped.size(line_height).height;
                if byte_offset <= line_range.end {
                    let display_offset = display_line.display_offset_for_raw(byte_offset - line_range.start);
                    let pos = wrapped.position_for_index(display_offset, line_height).unwrap_or(point(px(0.), px(0.)));
                    let x = pos.x + indent_px(display_line.indent.level());
                    let origin = element_bounds.origin + point(x, y_offset + pos.y);
                    return Some(Bounds { origin, size: size(px(1.), line_height) });
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
        // Would need this element's current screen bounds, which (unlike
        // `bounds_for_range`) this method isn't passed — mouse-driven cursor
        // placement already goes through `offset_for_position` in
        // `element.rs` instead, where `canvas`'s paint closure has `bounds`
        // directly in scope. This only affects secondary OS text-service
        // affordances (e.g. a right-click "Look Up"), not typing or IME
        // composition, so returning "unsupported" is an acceptable Phase 1
        // gap rather than plumbing a persisted-bounds field just for it.
        None
    }

    fn set_selected_text_range(&mut self, range_utf16: Range<usize>, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.buffer.text();
        let start = utf16_to_utf8(text, range_utf16.start);
        let end = utf16_to_utf8(text, range_utf16.end);
        self.buffer.set_cursor(start, false);
        self.buffer.set_cursor(end, true);
        cx.notify();
    }
}

// Deliberately not `use super::*` here: the production code above uses
// `use gpui_kit::*;`, which (with `test-support` enabled) also re-exports
// GPUI's `test` attribute macro — importing it transitively into this module
// would shadow the plain `#[test]` these pure-logic tests use and make it
// recurse infinitely (the same project-wide gotcha noted in `main.rs`).
#[cfg(test)]
mod tests {
    use super::{utf16_to_utf8, utf8_to_utf16};

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

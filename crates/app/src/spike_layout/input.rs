//! Platform text input and IME. For the spike the "document" the platform
//! sees is the cursor's line. Geometry comes from the frame snapshot only.

use std::ops::Range;

use gpui_kit::*;

use super::{LINE_HEIGHT, SpikeView};

fn utf16_to_utf8(text: &str, utf16_offset: usize) -> usize {
    let mut count = 0;
    for (byte, ch) in text.char_indices() {
        if count >= utf16_offset {
            return byte;
        }
        count += ch.len_utf16();
    }
    text.len()
}

fn utf8_to_utf16(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().map(char::len_utf16).sum()
}

impl SpikeView {
    fn line(&self) -> &str {
        &self.lines[self.cursor.0]
    }

    fn input_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        match range_utf16 {
            Some(r) => utf16_to_utf8(self.line(), r.start)..utf16_to_utf8(self.line(), r.end),
            None => self.marked.clone().unwrap_or(self.cursor.1..self.cursor.1),
        }
    }

    fn to_utf16(&self, range: Range<usize>) -> Range<usize> {
        utf8_to_utf16(self.line(), range.start)..utf8_to_utf16(self.line(), range.end)
    }
}

impl EntityInputHandler for SpikeView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.input_range(Some(range_utf16));
        *adjusted = Some(self.to_utf16(range.clone()));
        self.line().get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.to_utf16(self.cursor.1..self.cursor.1),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        self.marked = None;
        self.replace(range, text);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        let start = range.start;
        self.replace(range, new_text);
        self.marked = (!new_text.is_empty()).then(|| start..start + new_text.len());
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let byte = utf16_to_utf8(self.line(), range_utf16.start);
        let snapshot = self.snapshot.as_ref()?;
        let line = snapshot.lines.iter().find(|l| l.index == self.cursor.0)?;
        // The snapshot may be one edit older than the text.
        let byte = byte.min(line.wrapped.len());
        let at = line.wrapped.position_for_index(byte, LINE_HEIGHT)?;
        Some(Bounds::new(line.origin + at, size(px(1.), LINE_HEIGHT)))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let snapshot = self.snapshot.as_ref()?;
        let line = snapshot.lines.iter().find(|l| l.index == self.cursor.0)?;
        let byte = line
            .wrapped
            .closest_index_for_position(point - line.origin, LINE_HEIGHT)
            .unwrap_or_else(|nearest| nearest);
        Some(utf8_to_utf16(self.line(), byte.min(self.line().len())))
    }
}

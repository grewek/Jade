//! What the editor sets about how the document is shown, as opposed to what
//! the text says: the width regions lay out across, which region or table is
//! raw, the cell being edited, a divider drag's preview, justification and the
//! embedded images' sizes. Setters return whether the change alters row counts,
//! so the caller knows when to re-measure.

use std::ops::Range;
use std::sync::Arc;

use crate::text_editor::highlight::{Highlighted, Span};
use crate::text_editor::image::Sizes as ImageSizes;

#[derive(Default)]
pub(in crate::text_editor) struct ViewState {
    /// The width regions are laid out across. `None` lays every region out
    /// stacked; set by whoever builds the row counts (`RowCache`).
    column_width: Option<f32>,
    /// The region shown as source (Ctrl+/): stacked, its markers visible.
    raw_region: Option<usize>,
    /// The shares a divider drag is previewing for one region (its index and
    /// the shares), laid out in place of the source's until the drag ends.
    column_preview: Option<(usize, Vec<u8>)>,
    /// The table shown as raw text (Ctrl+/, or the caret on its delimiter
    /// row) instead of rendered; see `Editor::sync_table_view`.
    raw_table: Option<usize>,
    /// The table cell the caret is in, as `(line, column)`: it shows its text
    /// as typed and is what row counts and painting treat as being edited.
    editing_cell: Option<(usize, usize)>,
    /// Prose is justified: set from the settings each frame.
    justify: bool,
    /// Natural sizes of the images the note embeds, read off the main thread.
    images: ImageSizes,
    /// Highlighted code blocks near the viewport as `(content lines, spans)`,
    /// sorted; colour only, so setting them never changes a row count.
    code: Vec<(Range<usize>, Arc<Highlighted>)>,
}

impl ViewState {
    pub(in crate::text_editor) fn column_width(&self) -> Option<f32> {
        self.column_width
    }

    /// Sets the width regions are laid out across; `true` when it changed.
    pub(in crate::text_editor) fn set_column_width(&mut self, width: Option<f32>) -> bool {
        std::mem::replace(&mut self.column_width, width) != width
    }

    pub(in crate::text_editor) fn raw_region(&self) -> Option<usize> {
        self.raw_region
    }

    /// Shows region `index` as source; `true` when that changed, which changes
    /// every row count in it.
    pub(in crate::text_editor) fn set_raw_region(&mut self, index: Option<usize>) -> bool {
        std::mem::replace(&mut self.raw_region, index) != index
    }

    /// The region whose shares a divider drag is previewing.
    pub(in crate::text_editor) fn column_preview_region(&self) -> Option<usize> {
        self.column_preview.as_ref().map(|(index, _)| *index)
    }

    /// The previewed shares, if they are for region `index` of `count` columns.
    pub(super) fn preview_for(&self, index: usize, count: usize) -> Option<&[u8]> {
        self.column_preview
            .as_ref()
            .filter(|(i, s)| *i == index && s.len() == count)
            .map(|(_, shares)| shares.as_slice())
    }

    /// Sets the shares a divider drag previews; `true` when that changed what
    /// is laid out, which changes the region's height.
    pub(in crate::text_editor) fn set_column_preview(
        &mut self,
        preview: Option<(usize, Vec<u8>)>,
    ) -> bool {
        std::mem::replace(&mut self.column_preview, preview.clone()) != preview
    }

    pub(in crate::text_editor) fn raw_table(&self) -> Option<usize> {
        self.raw_table
    }

    pub(in crate::text_editor) fn editing_cell(&self) -> Option<(usize, usize)> {
        self.editing_cell
    }

    /// Sets the view state of the tables; `true` when the raw one changed, which
    /// changes every row count in two tables.
    pub(in crate::text_editor) fn set_table_view(
        &mut self,
        raw_table: Option<usize>,
        editing_cell: Option<(usize, usize)>,
    ) -> bool {
        self.editing_cell = editing_cell;
        std::mem::replace(&mut self.raw_table, raw_table) != raw_table
    }

    /// Whether prose is justified (a paint-only setting: no row count changes).
    pub(in crate::text_editor) fn justify(&self) -> bool {
        self.justify
    }

    pub(in crate::text_editor) fn set_justify(&mut self, on: bool) {
        self.justify = on;
    }

    /// The coloured tokens of code line `i`; empty when it has none.
    pub(in crate::text_editor) fn code_line(&self, i: usize) -> &[Span] {
        let at = self.code.partition_point(|(lines, _)| lines.end <= i);
        match self.code.get(at) {
            Some((lines, spans)) if lines.start <= i => {
                spans.lines.get(i - lines.start).map_or(&[], Vec::as_slice)
            }
            _ => &[],
        }
    }

    /// Sets the highlighted blocks (sorted, not overlapping); `true` when they
    /// differ, so the caller repaints.
    pub(in crate::text_editor) fn set_code(
        &mut self,
        blocks: Vec<(Range<usize>, Arc<Highlighted>)>,
    ) -> bool {
        debug_assert!(blocks.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let same = blocks.len() == self.code.len()
            && blocks
                .iter()
                .zip(&self.code)
                .all(|(a, b)| a.0 == b.0 && Arc::ptr_eq(&a.1, &b.1));
        self.code = blocks;
        !same
    }

    pub(in crate::text_editor) fn images(&self) -> &ImageSizes {
        &self.images
    }

    pub(in crate::text_editor) fn images_mut(&mut self) -> &mut ImageSizes {
        &mut self.images
    }
}

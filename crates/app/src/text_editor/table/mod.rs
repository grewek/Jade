//! Tables: the pipe-table model (parsing, widths), the box geometry of a
//! rendered cell, structure edits as text transforms, and the handle hit
//! rectangles. Editing the cell under the caret lives in `editor/table_edit.rs`
//! and painting in `element/table_paint.rs`, which need those modules' private
//! state.
//!
//! `model` and `geometry` are private: the editor uses their items as
//! `table::X`. `ops` and `handles` stay namespaced (`table::ops::apply`,
//! `table::handles::hover_at`), their names being too generic to flatten.

mod geometry;
pub(super) mod handles;
mod model;
pub(super) mod ops;

pub(super) use geometry::{
    CELL_PAD, align_fraction, box_width, caret_at, cell_at, cell_ranges, column_left, edit_range,
    layout_row_editing, layout_row_in, offset_at_editing, offset_in_line, text_left, x_in_line,
};
#[cfg(test)]
pub(super) use model::classify_tables;
pub(super) use model::{
    Align, MIN_COLUMN_WIDTH, Table, classify_table_list, classify_table_list_from, fits_unshrunk,
    is_pipe_row, resolve_widths, row_line_count, row_line_count_in, row_scale, table_index_at,
};

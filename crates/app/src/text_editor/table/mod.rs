//! Tables: the pipe-table model (parsing, widths), the box geometry of a
//! rendered cell, structure edits as text transforms, and the handle hit
//! rectangles. Editing the cell under the caret lives in `editor/table_edit.rs`
//! and painting in `element/table_paint.rs`, which need those modules' private
//! state.

pub(super) mod geometry;
pub(super) mod handles;
pub(super) mod model;
pub(super) mod ops;

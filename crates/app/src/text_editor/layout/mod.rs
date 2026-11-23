//! What decides how one line looks (`state`) and how it is turned into shaped
//! text (`shape`): the pure dispatch from a line to its `DisplayLine`s, the
//! monospace row arithmetic, and the one place text is actually shaped.
//! Row counting (`rows`) and painting sit on top of this.

mod shape;
mod state;

pub(super) use shape::{
    RowFontMetrics, build_display_lines, char_budget, entry_heights, fit_with_indent,
    formula_reach, line_runs, mono_char_width, shape_display_line, shape_entry, shaped_height,
    tall_rows, wrapped_row_count,
};
pub(super) use state::{
    LineState, MathPart, TableLineKind, callout_at_line, indent_px, is_table_active,
    table_line_kind,
};

#[cfg(test)]
pub(super) use state::is_active_line;

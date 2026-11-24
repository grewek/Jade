use std::ops::Range;

use gpui_kit::*;

use crate::text_editor::doc::DocumentCache;
use crate::text_editor::image::Sizes as ImageSizes;
use crate::text_editor::math::Macros;
use crate::text_editor::preview::LineContext;
use crate::text_editor::preview::LineRole;
use crate::text_editor::preview::callout::{Callout, callout_index_at};
use crate::text_editor::table::{Table, table_index_at};

/// Resolves `line_index`'s [`Callout`] (if it belongs to one at all) from
/// the resolved `callouts` — same shape as `table_line_kind`.
pub(in crate::text_editor) fn callout_at_line(
    callouts: &[Callout],
    line_index: usize,
) -> Option<&Callout> {
    callout_index_at(callouts, line_index).map(|idx| &callouts[idx])
}

/// Everything besides its text that decides how one line is displayed. Built
/// by [`LineState::of`] from the document's caches wherever a line is shaped
/// or measured (painting, hit-testing, row counts, the IME's caret bounds), so
/// they cannot disagree about what the line looks like.
pub(in crate::text_editor) struct LineState<'a> {
    pub role: LineRole,
    pub block_ctx: LineContext,
    pub table_kind: TableLineKind<'a>,
    pub table_active: bool,
    pub callout: Option<&'a Callout>,
    pub callout_is_header: bool,
    /// The cursor is on this line, so its Markdown is shown raw.
    pub active: bool,
    /// Raw byte ranges of links on the line whose target is missing.
    pub broken: &'a [Range<usize>],
    /// Sizes of the note's images, for an image block's height.
    pub images: Option<&'a ImageSizes>,
    /// A body line of a folded callout, or a line inside a region laid out side
    /// by side other than its first (its column draws it): no rows at all.
    pub hidden: bool,
    /// The column of this table row the caret is in: it shows its text as typed.
    pub editing_cell: Option<usize>,
    /// The open or close marker of a columns region: shows nothing unless active.
    pub column_frame: bool,
    /// Prose rows are stretched to the wrap width (see `justify.rs`).
    pub justify: bool,
    /// The line's part in a `$$` block shown as a formula.
    pub math: MathPart,
    /// The note's macros, which its formulas are expanded with.
    pub macros: &'a Macros,
}

/// Where a line is in a `$$` block that is shown as a formula (the caret is not
/// in it): the first line holds the whole block's rows and the rest none, like
/// a folded callout. A block with the caret in it is plain lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::text_editor) enum MathPart {
    None,
    /// The first line, over this many rows.
    Head(usize),
    Tail,
}

impl<'a> LineState<'a> {
    pub(in crate::text_editor) fn of(
        doc: &'a DocumentCache,
        i: usize,
        cursor: usize,
        focused: bool,
    ) -> Self {
        let line_range = doc.line_start_byte(i)..doc.line_end_byte(i);
        let callout = callout_at_line(doc.callouts(), i);
        let math = math_part(doc, i, cursor, focused);
        let column_hidden = doc
            .side_by_side(i)
            .is_some_and(|(region, _)| i != region.start_line);
        LineState {
            role: doc.line_roles()[i],
            block_ctx: doc.block_contexts()[i],
            table_kind: table_line_kind(doc.tables(), i),
            table_active: is_table_active(doc, i, focused),
            callout,
            callout_is_header: callout.is_some_and(|c| c.is_header(i)),
            active: is_active_line(&line_range, cursor, focused),
            broken: &[],
            images: Some(doc.view.images()),
            hidden: column_hidden || callout.is_some_and(|c| c.hides(i)) || math == MathPart::Tail,
            editing_cell: doc
                .view
                .editing_cell()
                .filter(|&(line, _)| line == i)
                .map(|(_, column)| column),
            column_frame: doc.is_column_frame(i),
            justify: doc.view.justify(),
            math,
            macros: doc.macros(),
        }
    }

    /// Line `i` as content of its column: laid out and painted there, so not
    /// hidden by being inside the region (a folded callout's body still is).
    pub(in crate::text_editor) fn in_column(
        doc: &'a DocumentCache,
        i: usize,
        cursor: usize,
        focused: bool,
    ) -> Self {
        let state = Self::of(doc, i, cursor, focused);
        LineState {
            hidden: state.callout.is_some_and(|c| c.hides(i)),
            // A column shows a block's lines as source.
            math: MathPart::None,
            ..state
        }
    }

    /// An ordinary line outside any block, for tests to vary with struct update.
    #[cfg(test)]
    pub(in crate::text_editor) fn plain() -> Self {
        LineState {
            role: LineRole::Plain,
            block_ctx: LineContext::Plain,
            table_kind: TableLineKind::None,
            table_active: false,
            callout: None,
            callout_is_header: false,
            active: false,
            broken: &[],
            images: None,
            hidden: false,
            editing_cell: None,
            column_frame: false,
            justify: false,
            math: MathPart::None,
            macros: &crate::text_editor::math::NO_MACROS,
        }
    }
}

/// Which table-related rendering a `LineRole::Plain` line needs, resolved
/// by the caller (which already knows the line's absolute index, needed to
/// tell a table's delimiter row from its header/body rows) — see
/// `table_line_kind`.
pub(in crate::text_editor) enum TableLineKind<'a> {
    None,
    Delimiter,
    Row(&'a Table),
}

/// Resolves `line_index`'s [`TableLineKind`] from the resolved `tables`.
pub(in crate::text_editor) fn table_line_kind(
    tables: &[Table],
    line_index: usize,
) -> TableLineKind<'_> {
    match table_index_at(tables, line_index) {
        None => TableLineKind::None,
        Some(idx) => {
            let table = &tables[idx];
            if table.is_delimiter(line_index) {
                TableLineKind::Delimiter
            } else {
                TableLineKind::Row(table)
            }
        }
    }
}

/// Whether `line_index`'s table (if any) shows its raw source instead of the
/// rendered box: only in raw mode (`DocumentCache::raw_table`, set by Ctrl+/ or
/// the caret on the delimiter row). Otherwise a table with the caret in it stays
/// rendered and its cell under the caret is edited in place.
pub(in crate::text_editor) fn is_table_active(
    doc: &DocumentCache,
    line_index: usize,
    focused: bool,
) -> bool {
    focused
        && doc
            .view
            .raw_table()
            .is_some_and(|raw| table_index_at(doc.tables(), line_index) == Some(raw))
}

/// Pixel width of one structural indent level (see
/// `preview::LineIndent`) — applied identically by `paint_lines`,
/// `offset_for_position`, and (via [`indent_px`]) `input_handler.rs`'s
/// `bounds_for_range`, so a line's painted position and its hit-testing/IME
/// math always agree.
const INDENT_UNIT: Pixels = px(20.);

/// The pixel x-offset `level` structural indent levels shifts a line's
/// content by — `level` is [`LineIndent::level`], `0` for a line with no
/// indent.
pub(in crate::text_editor) fn indent_px(level: u8) -> Pixels {
    INDENT_UNIT * level as f32
}

/// Whether the line spanning `line_range` should render raw/unstyled rather
/// than with Live Preview markup hidden: only while `focused` (the editor
/// actually has focus — a freshly opened note has an unfocused editor whose
/// `EditorBuffer::cursor()` still defaults to byte `0`, which would
/// otherwise permanently pin line 0 as "active" even though no editing is
/// happening) *and* `cursor` sits inside `line_range` (inclusive of both
/// ends, so a cursor resting right at a line's last byte, before its `\n`,
/// still counts).
pub(in crate::text_editor) fn is_active_line(
    line_range: &std::ops::Range<usize>,
    cursor: usize,
    focused: bool,
) -> bool {
    focused && cursor >= line_range.start && cursor <= line_range.end
}

/// Line `i`'s part in a `$$` block shown as a formula.
fn math_part(doc: &DocumentCache, i: usize, cursor: usize, focused: bool) -> MathPart {
    match doc.math_block_at(i) {
        Some(b) if doc.math_block_active(b, cursor, focused) => MathPart::None,
        Some(b) => match b.rows() {
            Some(rows) if b.first_line == i => MathPart::Head(rows),
            Some(_) => MathPart::Tail,
            // Not a formula (yet): its source, as typed.
            None => MathPart::None,
        },
        None => MathPart::None,
    }
}

//! Painting a rendered table: its box, grid, the edited cell and the controls on
//! its edge. Moves into the table module with the rest of table code.

use super::*;
#[allow(unused_imports)]
use crate::text_editor::math::Macros;
use crate::text_editor::table;

/// Corner radius of a table's box.
pub(super) const TABLE_RADIUS: Pixels = px(6.);

/// `trace` gets each box's extent, to check it against the lines it frames.
pub(super) fn paint_table_boxes(
    range: &Range<usize>,
    view: &FrameView,
    doc: &DocumentCache,
    palette: &crate::theme::Palette,
    (window, cx, trace): (&mut Window, &mut App, Option<&mut PaintTrace>),
) {
    let mut trace = trace;
    let tables = doc.tables();
    let FrameView {
        layout,
        focused,
        bounds,
        font,
        font_size,
        text_color,
        ..
    } = view;
    let first = tables.partition_point(|t| t.end_line <= range.start);
    for (offset, table) in tables[first..]
        .iter()
        .take_while(|t| t.start_line < range.end)
        .enumerate()
    {
        // A table in a side-by-side region is drawn with its column.
        if is_table_active(doc, table.start_line, *focused)
            || doc.side_by_side(table.start_line).is_some()
        {
            continue;
        }
        let index = first + offset;
        let columns = TableColumns::of(
            table,
            bounds.size.width,
            (font, *font_size, *text_color),
            window,
        );
        let at = |line: usize| layout.screen_y(bounds.origin.y, line);
        if let Some(trace) = trace.as_deref_mut() {
            let (top, bottom) = (at(table.start_line), at(table.end_line));
            trace.table_band(
                table.start_line..table.end_line,
                (f32::from(top), f32::from(bottom)),
            );
        }
        let visible_rows = range.start.saturating_sub(1)..range.end;
        paint_table_box(
            table,
            &columns,
            bounds.origin.x,
            &at,
            visible_rows,
            palette,
            window,
        );
        let hovered = view.table_hover.as_ref().filter(|(t, _)| *t == index);
        let dragging = view.table_drag.filter(|d| d.table == index);
        if hovered.is_some() || dragging.is_some() {
            let (grid, top) = grid_for(table, &columns, &at);
            let origin = point(bounds.origin.x, top);
            let shown = hovered.map_or(&[][..], |(_, handles)| handles.as_slice());
            let colors = handle_colors(palette, *text_color);
            paint_table_handles(&grid, origin, (shown, dragging), &colors, (window, cx));
        }
    }
}

pub(super) fn handle_colors(palette: &crate::theme::Palette, icon: Hsla) -> HandleColors {
    HandleColors {
        accent: palette.color(Role::Accent),
        on_accent: palette.color(Role::TextOnAccent),
        fill: palette.color(Role::TableHeader),
        border: palette.color(Role::BorderStrong),
        icon,
    }
}

/// One rendered table's box: header tint, column dividers, row separators and
/// the outline. `at` gives the window y of a line's top (the table's end line
/// for its bottom); `visible_rows` limits the row separators to what is on
/// screen.
pub(super) fn paint_table_box(
    table: &Table,
    columns: &TableColumns,
    left: Pixels,
    at: &dyn Fn(usize) -> Pixels,
    visible_rows: Range<usize>,
    palette: &crate::theme::Palette,
    window: &mut Window,
) {
    let strong = palette.color(Role::BorderStrong);
    let light = palette.color(Role::Border);
    let (top, bottom) = (at(table.start_line), at(table.end_line));
    let width = columns.box_px();
    let header_bottom = at(table.start_line + 2).min(bottom);
    let line = |x: Pixels, y: Pixels, w: Pixels, h: Pixels, color: Hsla| {
        fill(
            Bounds {
                origin: point(x, y),
                size: size(w, h),
            },
            color,
        )
    };
    window.paint_quad(quad(
        Bounds {
            origin: point(left, top),
            size: size(width, header_bottom - top),
        },
        Corners {
            top_left: TABLE_RADIUS,
            top_right: TABLE_RADIUS,
            bottom_left: px(0.),
            bottom_right: px(0.),
        },
        palette.color(Role::TableHeader),
        px(0.),
        Hsla::transparent_black(),
        BorderStyle::Solid,
    ));
    for column in 1..columns.widths.len() {
        let x = left + columns.x(table::column_left(&columns.widths, column));
        window.paint_quad(line(x, top, TABLE_LINE_WIDTH, bottom - top, light));
    }
    // Between body rows, in the visible part only.
    let rows =
        (table.start_line + 2).max(visible_rows.start)..(table.end_line - 1).min(visible_rows.end);
    for row in rows {
        let y = at(row + 1) - TABLE_LINE_WIDTH;
        window.paint_quad(line(left, y, width, TABLE_LINE_WIDTH, light));
    }
    window.paint_quad(line(
        left,
        header_bottom - TABLE_LINE_WIDTH,
        width,
        TABLE_LINE_WIDTH,
        strong,
    ));
    window.paint_quad(quad(
        Bounds {
            origin: point(left, top),
            size: size(width, bottom - top),
        },
        TABLE_RADIUS,
        Hsla::transparent_black(),
        TABLE_LINE_WIDTH,
        strong,
        BorderStyle::Solid,
    ));
}

/// Colors of the controls on a table's edge.
pub(super) struct HandleColors {
    pub(super) accent: Hsla,
    pub(super) on_accent: Hsla,
    pub(super) fill: Hsla,
    pub(super) border: Hsla,
    pub(super) icon: Hsla,
}

/// Draws a table's edge controls: a filled `+` circle, a bordered grip (filled
/// with the accent while it is dragged), and while dragging the drop line.
pub(super) fn paint_table_handles(
    grid: &Grid,
    origin: Point<Pixels>,
    (shown, dragging): (&[Handle], Option<TableDrag>),
    colors: &HandleColors,
    (window, cx): (&mut Window, &mut App),
) {
    let at = |handle: Handle| {
        let (x, y, w, h) = table::handles::rect_of(grid, handle);
        Bounds {
            origin: origin + point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    };
    let draw = |handle: Handle, window: &mut Window, cx: &mut App| {
        let (is_add, icon) = match handle {
            Handle::AddColumn(_) | Handle::AddRow(_) => (true, "icons/plus.svg"),
            Handle::ColumnGrip(_) => (false, "icons/grip-horizontal.svg"),
            Handle::RowGrip(_) => (false, "icons/grip-vertical.svg"),
        };
        let active = dragging.is_some_and(|d| d.grip == handle);
        paint_control(at(handle), icon, (is_add, active), colors, (window, cx));
    };
    // While dragging, only the dragged grip is drawn (and the drop line).
    let drawn: Vec<Handle> = match dragging {
        Some(drag) => vec![drag.grip],
        None => shown.to_vec(),
    };
    for handle in drawn {
        draw(handle, window, cx);
    }
    if let Some(drag) = dragging {
        let thickness = px(2.);
        let line = match drag.grip {
            Handle::ColumnGrip(_) => Bounds {
                origin: origin + point(px(grid.col_x[drag.over]) - thickness / 2.0, px(0.)),
                size: size(thickness, px(grid.row_y[grid.row_y.len() - 1])),
            },
            _ => Bounds {
                origin: origin + point(px(0.), px(grid.row_y[drag.over]) - thickness / 2.0),
                size: size(px(grid.col_x[grid.col_x.len() - 1]), thickness),
            },
        };
        window.paint_quad(fill(line, colors.accent));
    }
}

/// One table row's cell text, each cell's lines placed in its column by the
/// table's alignment. `origin` is the row's top-left; `header` makes it bold.
/// A drawn formula taller than a row makes each cell line `scale` rows high
/// (see `table::row_scale`); the text sits in the middle of them.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_table_row(
    line: &str,
    columns: &TableColumns,
    origin: Point<Pixels>,
    (header, editing, scale): (bool, Option<usize>, usize),
    (font, font_size, line_height): (&Font, Pixels, Pixels),
    (colors, widget_colors, macros): (&InlineColors, &WidgetColors, &Macros),
    window: &mut Window,
    cx: &mut App,
) {
    let pitch = line_height * scale as f32;
    let lift = (pitch - line_height) / 2.;
    for (c, cell) in table::layout_row_in(line, &columns.widths, editing, macros)
        .iter()
        .enumerate()
    {
        let column_width = columns.x(columns.widths[c] as f32);
        let left = origin.x + columns.x(table::text_left(&columns.widths, c));
        // The cell being edited is always left-aligned, so typing starts at its
        // left edge whatever the column's alignment.
        let fraction = if editing == Some(c) {
            0.0
        } else {
            table::align_fraction(columns.aligns[c])
        };
        for (j, range) in cell.lines.iter().enumerate() {
            if range.is_empty() {
                continue;
            }
            let text = &line[range.clone()];
            let shown = if editing == Some(c) {
                raw_cell_line(text)
            } else {
                build_cell_line_with(text, macros, columns.widths[c])
            };
            let wrapped = shape_entry(
                &shown,
                None,
                (font, font_size),
                colors,
                (&[], None),
                header,
                window,
            );
            let spare = (column_width - wrapped.size(line_height).width).max(px(0.));
            let at = point(left + spare * fraction, origin.y + pitch * j as f32 + lift);
            let _ = wrapped.paint(at, line_height, TextAlign::Left, None, window, cx);
            let entry = EntryHeight::uniform(line_height);
            let widgets = WidgetPaint {
                colors: widget_colors,
                inline: colors,
                font_size,
                entry: &entry,
                line_text: text,
                row_width: column_width,
                macros,
            };
            paint_widgets(&shown, &wrapped, at, &widgets, (window, cx));
        }
    }
}

/// The cell being edited and where its row is.
pub(super) struct EditingCell<'a> {
    pub(super) line: &'a str,
    /// Byte offset of `line` in the buffer.
    pub(super) line_start: usize,
    pub(super) column: usize,
    /// The row's top left.
    pub(super) origin: Point<Pixels>,
    pub(super) line_height: Pixels,
    /// Rows each cell line takes (see [`paint_table_row`]).
    pub(super) scale: usize,
    /// The note's macros: the other cells' formulas decide the row's height.
    pub(super) macros: &'a Macros,
}

/// Marks the cell being edited: an accent outline, the part of the selection
/// inside it and the caret, over the text `paint_table_row` drew.
pub(super) fn paint_editing_cell(
    cell: &EditingCell,
    columns: &TableColumns,
    (selection, caret): (Option<&Range<usize>>, Option<usize>),
    (accent, selection_fill, cursor_color): (Hsla, Hsla, Hsla),
    window: &mut Window,
) {
    let EditingCell {
        line,
        line_start,
        column,
        origin,
        line_height,
        scale,
        macros,
    } = *cell;
    let pitch = line_height * scale as f32;
    let lift = (pitch - line_height) / 2.;
    let lines = &table::layout_row_editing(line, &columns.widths, Some(column))[column].lines;
    let height = pitch
        * table::layout_row_in(line, &columns.widths, Some(column), macros)
            .iter()
            .map(|c| c.lines.len())
            .max()
            .unwrap_or(1) as f32;
    let left = origin.x + columns.x(table::column_left(&columns.widths, column));
    let width = columns.x(columns.widths[column] as f32 + 2.0 * table::CELL_PAD);
    window.paint_quad(quad(
        Bounds {
            origin: point(left, origin.y),
            size: size(width, height),
        },
        px(0.),
        Hsla::transparent_black(),
        CELL_OUTLINE,
        accent,
        BorderStyle::Solid,
    ));
    let x_of = |range: &Range<usize>, offset: usize| {
        columns.x(table::x_in_line(
            line,
            &columns.widths,
            column,
            range,
            offset,
        ))
    };
    if let Some(selection) = selection {
        let (from, to) = (
            selection.start.saturating_sub(line_start),
            selection.end.saturating_sub(line_start),
        );
        for (row, range) in lines.iter().enumerate() {
            let (a, b) = (from.max(range.start), to.min(range.end));
            if a < b {
                window.paint_quad(fill(
                    Bounds {
                        origin: point(
                            origin.x + x_of(range, a),
                            origin.y + pitch * row as f32 + lift,
                        ),
                        size: size(x_of(range, b) - x_of(range, a), line_height),
                    },
                    selection_fill,
                ));
            }
        }
    }
    if let Some(caret) = caret.filter(|c| *c >= line_start) {
        let offset = caret - line_start;
        let (row, x) = table::caret_at(line, &columns.widths, column, offset);
        window.paint_quad(fill(
            Bounds {
                origin: point(
                    origin.x + columns.x(x),
                    origin.y + pitch * row as f32 + lift,
                ),
                size: size(px(1.), line_height),
            },
            cursor_color,
        ));
    }
}

/// Width of the outline around the cell being edited.
pub(super) const CELL_OUTLINE: Pixels = px(1.5);

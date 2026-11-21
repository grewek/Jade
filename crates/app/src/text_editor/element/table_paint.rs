//! Painting a rendered table: its box, grid, the edited cell and the controls on
//! its edge. Moves into the table module with the rest of table code.

use super::*;
#[allow(unused_imports)]
use crate::text_editor::table;

/// Corner radius of a table's box.
pub(super) const TABLE_RADIUS: Pixels = px(6.);

pub(super) fn paint_table_boxes(
    range: &Range<usize>,
    view: &FrameView,
    doc: &DocumentCache,
    palette: &crate::theme::Palette,
    (window, cx): (&mut Window, &mut App),
) {
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
        let at = |line: usize| bounds.origin.y + layout.cumulative_at(line);
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
/// One control: a round accent button (`round`) or a bordered grip, accent
/// while `active`.
/// The "back to columns" button of a region shown stacked, with the shares it
/// would set. It sits in the (empty) row of the region's open marker, at the
/// right. `None` for a region laid out as columns, shown as source, or too
/// narrow for its columns even at their default shares.
pub(super) fn restore_pill(
    view: &FrameView,
    doc: &DocumentCache,
    index: usize,
) -> Option<(Bounds<Pixels>, Vec<u8>)> {
    let region = *doc.columns().get(index)?;
    if region.tail
        || doc.raw_region() == Some(index)
        || doc.side_by_side(region.start_line).is_some()
    {
        return None;
    }
    let percent = columns::restored(&region, doc.column_width()?)?;
    let top = view.bounds.origin.y + view.layout.cumulative_at(region.start_line);
    let (w, h) = (px(24.), px(18.));
    let origin = point(
        view.bounds.origin.x + view.bounds.size.width - w - px(4.),
        top + (view.line_height - h) / 2.,
    );
    Some((
        Bounds {
            origin,
            size: size(w, h),
        },
        percent,
    ))
}

/// What a stacked region shows of its columns: the divider being dragged as a
/// ghost at the shares it would take (so it can be dragged back), else the
/// button that brings the columns back.
pub(super) fn paint_stacked_controls(
    view: &FrameView,
    doc: &DocumentCache,
    colors: &HandleColors,
    (window, cx): (&mut Window, &mut App),
) {
    let Some(total) = doc.column_width() else {
        return;
    };
    if let Some(drag) = &view.column_drag {
        let Some(&region) = doc.columns().get(drag.region) else {
            return;
        };
        if doc.side_by_side(region.start_line).is_some() {
            return;
        }
        let top = view.bounds.origin.y + view.layout.cumulative_at(region.start_line);
        let bottom = view.bounds.origin.y + view.layout.cumulative_at(region.end_line);
        let x = view.bounds.origin.x
            + px(columns::divider_x_unchecked(
                total,
                &region,
                &drag.percent,
                drag.divider,
            ));
        let line = Bounds {
            origin: point(x - px(1.), top),
            size: size(px(2.), bottom - top),
        };
        window.paint_quad(fill(line, colors.accent));
        let grip = Bounds {
            origin: point(
                x - px(columns::handles::GRIP_SHORT) / 2.,
                top + (px(columns::handles::GRIP_LONG)).min((bottom - top) / 2.)
                    - px(columns::handles::GRIP_LONG) / 2.,
            ),
            size: size(
                px(columns::handles::GRIP_SHORT),
                px(columns::handles::GRIP_LONG),
            ),
        };
        paint_control(
            grip,
            "icons/grip-vertical.svg",
            (false, true),
            colors,
            (window, cx),
        );
        return;
    }
    for index in 0..doc.columns().len() {
        let Some((bounds, _)) = restore_pill(view, doc, index) else {
            continue;
        };
        if bounds.origin.y + bounds.size.height < view.viewport.origin.y
            || bounds.origin.y > view.viewport.origin.y + view.viewport.size.height
        {
            continue;
        }
        paint_control(
            bounds,
            "icons/columns-2.svg",
            (false, false),
            colors,
            (window, cx),
        );
    }
}

pub(super) fn paint_control(
    bounds: Bounds<Pixels>,
    icon: &'static str,
    (round, active): (bool, bool),
    colors: &HandleColors,
    (window, cx): (&mut Window, &mut App),
) {
    let filled = round || active;
    let (fill, border, glyph) = if filled {
        (colors.accent, Hsla::transparent_black(), colors.on_accent)
    } else {
        (colors.fill, colors.border, colors.icon)
    };
    let radius = if round {
        bounds.size.width / 2.0
    } else {
        px(4.)
    };
    let edge = if filled { px(0.) } else { px(1.) };
    window.paint_quad(quad(bounds, radius, fill, edge, border, BorderStyle::Solid));
    let inner = if round { px(10.) } else { px(8.) };
    let glyph_bounds = Bounds {
        origin: bounds.origin
            + point(
                (bounds.size.width - inner) / 2.0,
                (bounds.size.height - inner) / 2.0,
            ),
        size: size(inner, inner),
    };
    paint_icon(icon, glyph_bounds, glyph, (window, cx));
}

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
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_table_row(
    line: &str,
    columns: &TableColumns,
    origin: Point<Pixels>,
    (header, editing): (bool, Option<usize>),
    (font, font_size, line_height): (&Font, Pixels, Pixels),
    colors: &InlineColors,
    window: &mut Window,
    cx: &mut App,
) {
    for (c, cell) in layout_row_editing(line, &columns.widths, editing)
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
            let shown = if editing == Some(c) {
                raw_cell_line(&line[range.clone()])
            } else {
                build_cell_line(&line[range.clone()])
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
            let at = point(left + spare * fraction, origin.y + line_height * j as f32);
            let _ = wrapped.paint(at, line_height, TextAlign::Left, None, window, cx);
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
    } = *cell;
    let lines = &table::layout_row_editing(line, &columns.widths, Some(column))[column].lines;
    let height = line_height
        * table::layout_row_editing(line, &columns.widths, Some(column))
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
                            origin.y + line_height * row as f32,
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
                origin: point(origin.x + columns.x(x), origin.y + line_height * row as f32),
                size: size(px(1.), line_height),
            },
            cursor_color,
        ));
    }
}

/// Width of the outline around the cell being edited.
pub(super) const CELL_OUTLINE: Pixels = px(1.5);

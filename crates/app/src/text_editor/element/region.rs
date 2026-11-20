//! Painting and hit-testing a columns region laid out side by side (see
//! `columns.rs`). The region is one block in the row cache: its first line has
//! the height of its tallest column and the rest have none, so everything here
//! is placed from that line's top. A column's lines are shaped at the column's
//! width ([`shape_region`], used by painting and by clicks, so both see the same
//! rows) and stacked down the column.
//!
//! Not here yet: the caret's Up/Down across columns, autoscroll to a caret in
//! a column, IME bounds in a column, and table controls in a column.

use super::*;

/// One shaped line of a column: `y` is its top below the region's top.
pub(super) struct ShapedLine {
    pub line: usize,
    pub y: Pixels,
    pub height: Pixels,
    pub results: Vec<(WrappedLine, DisplayLine)>,
    /// Raw ranges of links on the line whose target is missing.
    pub broken: Vec<Range<usize>>,
}

/// One column: `x` and `width` are relative to the editor's left edge.
pub(super) struct ShapedColumn {
    pub x: Pixels,
    pub width: Pixels,
    pub lines: Vec<ShapedLine>,
}

pub(super) struct ShapedRegion {
    pub columns: Vec<ShapedColumn>,
    /// The region's height in rows: its tallest column, at least one.
    pub rows: usize,
}

/// Shapes every column of `region` at its own width. Cost is the region's
/// lines, shaped once per call.
pub(super) fn shape_region(
    text: &str,
    doc: &DocumentCache,
    (region, split): (&Columns, &columns::Split),
    (cursor, focused): (usize, bool),
    (font, font_size, line_height): (&Font, Pixels, Pixels),
    (colors, links): (InlineColors, Option<&LinkTable>),
    window: &mut Window,
) -> ShapedRegion {
    let mut rows = 1;
    let mut shaped = Vec::with_capacity(region.count);
    for k in 0..region.count {
        let width = px(split.width(k));
        let mut lines = Vec::with_capacity(region.column_lines(k).len());
        let mut y = px(0.);
        for i in region.column_lines(k) {
            let broken: Vec<Range<usize>> = links
                .map(|l| l.broken_on_line(i).collect())
                .unwrap_or_default();
            let state = column_state(doc, i, (cursor, focused), &broken);
            let line_text = &text[doc.line_start_byte(i)..doc.line_end_byte(i)];
            let results =
                shape_display_line(line_text, &state, font, font_size, colors, width, window);
            let height: Pixels = results
                .iter()
                .map(|(wrapped, _)| wrapped.size(line_height).height)
                .sum();
            lines.push(ShapedLine {
                line: i,
                y,
                height,
                results,
                broken,
            });
            y += height;
        }
        rows = rows.max((y / line_height).round() as usize);
        shaped.push(ShapedColumn {
            x: px(split.x(k)),
            width,
            lines,
        });
    }
    ShapedRegion {
        columns: shaped,
        rows,
    }
}

fn column_state<'a>(
    doc: &'a DocumentCache,
    i: usize,
    (cursor, focused): (usize, bool),
    broken: &'a [Range<usize>],
) -> LineState<'a> {
    LineState {
        broken,
        ..LineState::in_column(doc, i, cursor, focused)
    }
}

/// The frame a column paints in: the editor's top, the column's left and width.
fn column_bounds(bounds: Bounds<Pixels>, column: &ShapedColumn) -> Bounds<Pixels> {
    Bounds {
        origin: point(bounds.origin.x + column.x, bounds.origin.y),
        size: size(column.width, bounds.size.height),
    }
}

/// Paints the region whose first line is at `top` below `bounds.origin.y`:
/// dividers, then per column the callout bands and table boxes (which go under
/// the text), then the lines.
pub(super) fn paint_region(
    env: &PaintEnv,
    shaped: &ShapedRegion,
    bounds: Bounds<Pixels>,
    top: Pixels,
    (window, cx): (&mut Window, &mut App),
) {
    let doc = env.doc;
    let height = env.line_height * shaped.rows as f32;
    let divider = env.palette.color(Role::Border);
    for pair in shaped.columns.windows(2) {
        let middle = (pair[0].x + pair[0].width + pair[1].x) / 2.;
        window.paint_quad(fill(
            Bounds {
                origin: point(
                    bounds.origin.x + middle - TABLE_LINE_WIDTH / 2.,
                    bounds.origin.y + top,
                ),
                size: size(TABLE_LINE_WIDTH, height),
            },
            divider,
        ));
    }
    for column in &shaped.columns {
        let frame = column_bounds(bounds, column);
        paint_column_blocks(env, column, frame, top, (window, cx));
        let mut tables = None;
        for sl in column.lines.iter().filter(|sl| !sl.results.is_empty()) {
            let state = column_state(doc, sl.line, (env.cursor, env.show_cursor), &sl.broken);
            let range = doc.line_start_byte(sl.line)..doc.line_end_byte(sl.line);
            let slot = LineSlot {
                i: sl.line,
                line_text: &env.text[range.clone()],
                line_range: range,
                state: &state,
                results: &sl.results,
                bounds: frame,
                y_offset: top + sl.y,
                total_height: sl.height,
            };
            paint_line(env, &slot, &mut tables, window, cx);
        }
    }
}

/// The callout bands and table boxes of one column, from its lines' places.
fn paint_column_blocks(
    env: &PaintEnv,
    column: &ShapedColumn,
    frame: Bounds<Pixels>,
    top: Pixels,
    (window, cx): (&mut Window, &mut App),
) {
    let _ = cx;
    let doc = env.doc;
    let at = |line: usize| {
        let y = column.lines.iter().find(|sl| sl.line >= line).map_or_else(
            || column.lines.last().map_or(px(0.), |sl| sl.y + sl.height),
            |sl| sl.y,
        );
        frame.origin.y + top + y
    };
    let span = (frame.origin.x, frame.size.width);
    let mut last_callout = None;
    let mut last_table = None;
    for sl in &column.lines {
        if let Some(index) = callout_index_at(doc.callouts(), sl.line)
            && last_callout != Some(index)
        {
            last_callout = Some(index);
            let callout = &doc.callouts()[index];
            let end = column
                .lines
                .iter()
                .rfind(|l| l.line < callout.end_line)
                .map_or(sl.y, |l| l.y + l.height);
            let (band_top, bottom) = (at(callout.start_line), frame.origin.y + top + end);
            paint_callout_band(callout, (band_top, bottom), span, env.palette, window);
        }
        if let Some(index) = table_index_at(doc.tables(), sl.line)
            && last_table != Some(index)
        {
            last_table = Some(index);
            let table = &doc.tables()[index];
            if is_table_active(doc, table.start_line, env.show_cursor) {
                continue;
            }
            let columns = TableColumns::of(
                table,
                frame.size.width,
                (env.font, env.font_size, env.text_color),
                window,
            );
            let table_at = |line: usize| {
                if line >= table.end_line {
                    at(line.max(table.end_line))
                } else {
                    at(line)
                }
            };
            let rows = table.start_line..table.end_line;
            paint_table_box(
                table,
                &columns,
                frame.origin.x,
                &table_at,
                rows,
                env.palette,
                window,
            );
        }
    }
}

/// What a click needs to place itself in a line.
pub(super) struct HitFrame<'a> {
    pub bounds: Bounds<Pixels>,
    /// The click's y below the editor's top.
    pub relative_y: Pixels,
    pub position: Point<Pixels>,
    pub line_height: Pixels,
    pub font: &'a Font,
    pub font_size: Pixels,
    pub text_color: Hsla,
}

/// The click at `frame.position`, known to be in the region whose first line
/// is at `top`: the column nearest it, and in it the line under the point (the
/// end of the column's last line when the point is below its text).
pub(super) fn hit(
    doc: &DocumentCache,
    text: &str,
    shaped: &ShapedRegion,
    region: &Columns,
    (top, (cursor, focused)): (Pixels, (usize, bool)),
    frame: &HitFrame,
    window: &mut Window,
) -> Hit {
    let x = frame.position.x - frame.bounds.origin.x;
    let distance = |c: &&ShapedColumn| {
        if x < c.x {
            c.x - x
        } else {
            (x - (c.x + c.width)).max(px(0.))
        }
    };
    let Some(column) = shaped.columns.iter().min_by(|a, b| {
        distance(a)
            .partial_cmp(&distance(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return Hit::at(doc.line_start_byte(region.start_line));
    };
    let bounds = column_bounds(frame.bounds, column);
    let column_frame = HitFrame { bounds, ..*frame };
    let mut last = None;
    for sl in column.lines.iter().filter(|sl| !sl.results.is_empty()) {
        let range = doc.line_start_byte(sl.line)..doc.line_end_byte(sl.line);
        let state = column_state(doc, sl.line, (cursor, focused), &sl.broken);
        let line_text = &text[range.clone()];
        let found = hit_in_line(
            doc,
            (sl.line, line_text, range.start),
            (&state, &sl.results),
            top + sl.y,
            &column_frame,
            window,
        );
        match found {
            Ok(hit) => return hit,
            Err(_) => last = Some(range.end),
        }
    }
    // Below the column's text, or an empty column: the end of its last line,
    // else the blank line right after the open marker.
    Hit::at(last.unwrap_or_else(|| doc.line_start_byte(region.start_line + 1)))
}

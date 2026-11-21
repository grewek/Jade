//! Painting and hit-testing a columns region laid out side by side (see
//! `columns.rs`). The region is one block in the row cache: its first line has
//! the height of its tallest column and the rest have none, so everything here
//! is placed from that line's top. A column's lines are shaped at the column's
//! width ([`shape_region`], used by painting and by clicks, so both see the same
//! rows) and stacked down the column.
//!
//! The caret's Up/Down across columns lives in `editor/column_edit.rs`.

use super::*;

/// One shaped line of a column: `y` is its top below the region's top.
pub(super) struct ShapedLine {
    pub line: usize,
    pub y: Pixels,
    pub height: Pixels,
    pub results: Vec<(WrappedLine, DisplayLine)>,
    /// Raw ranges of links on the line whose target is missing.
    pub broken: Vec<Range<usize>>,
    /// Where this line's text sits, from the editor's left edge: its column's
    /// for ordinary columns, but text flowing around a float is narrower beside
    /// it than below it, line by line.
    pub x: Pixels,
    pub width: Pixels,
    /// The part of the raw line this entry shows: all of it, except that a line
    /// crossing the bottom of a float is two entries, the rows beside the float
    /// and the rest below it.
    pub raw: Range<usize>,
}

impl ShapedLine {
    /// The frame this entry is painted in: `bounds` is the editor's.
    pub(super) fn frame(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: point(bounds.origin.x + self.x, bounds.origin.y),
            size: size(self.width, bounds.size.height),
        }
    }

    /// The raw byte range in the document, given the line's start.
    pub(super) fn raw_in_doc(&self, line_start: usize) -> Range<usize> {
        line_start + self.raw.start..line_start + self.raw.end
    }
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
    /// The line's whole width.
    pub total: Pixels,
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
    if split.aside {
        return shape_aside(
            text,
            doc,
            (region, split),
            (cursor, focused),
            (font, font_size, line_height),
            (colors, links),
            window,
        );
    }
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
                x: px(split.x(k)),
                width,
                raw: 0..line_text.len(),
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
        total: px(split.total),
    }
}

/// One line shaped at `width`, with its broken links and total height.
fn shape_at(
    text: &str,
    doc: &DocumentCache,
    i: usize,
    width: Pixels,
    (cursor, focused): (usize, bool),
    (font, font_size, colors): (&Font, Pixels, InlineColors),
    (links, window): (Option<&LinkTable>, &mut Window),
) -> (Vec<(WrappedLine, DisplayLine)>, Vec<Range<usize>>, Pixels) {
    let broken: Vec<Range<usize>> = links
        .map(|l| l.broken_on_line(i).collect())
        .unwrap_or_default();
    let state = column_state(doc, i, (cursor, focused), &broken);
    let line_text = &text[doc.line_start_byte(i)..doc.line_end_byte(i)];
    let results = shape_display_line(line_text, &state, font, font_size, colors, width, window);
    let height = results
        .iter()
        .map(|(w, _)| w.size(window.line_height()).height)
        .sum();
    (results, broken, height)
}

/// Whether a line is prose that may wrap partly beside a float and partly
/// below it: nothing with its own box, marker or indent.
fn flowable(state: &LineState, display_line: &DisplayLine, line_text: &str) -> bool {
    matches!(state.role, LineRole::Plain)
        && matches!(state.table_kind, TableLineKind::None)
        && state.callout.is_none()
        && !state.callout_is_header
        && !state.column_frame
        && display_line.indent == LineIndent::None
        && image_block(line_text, state).is_none()
        && !display_line.segments.iter().any(|s| {
            s.is_replaced() || matches!(s.style, InlineStyle::Heading(_) | InlineStyle::Widget(_))
        })
}

/// Which side's floats a band is cut for.
const LEFT: usize = 0;
const RIGHT: usize = 1;

/// The raw ranges in `broken` that fall in `piece`, rebased to it.
fn broken_in(broken: &[Range<usize>], piece: &Range<usize>) -> Vec<Range<usize>> {
    broken
        .iter()
        .filter(|r| r.end > piece.start && r.start < piece.end)
        .map(|r| r.start.max(piece.start) - piece.start..r.end.min(piece.end) - piece.start)
        .collect()
}

/// Where the text may run at one row of an aside: the floats stacked down a
/// side keep that side's rows (and a row of air after them) to themselves.
struct Bands {
    /// Where each float is: its side and the rows it takes, including the air
    /// below it. Floats on one side never overlap (a later one waits below).
    placed: Vec<Placed>,
    /// Each side's width with its gap to the text, in pixels.
    reserved: [Pixels; 2],
    total: Pixels,
}

/// A float's side (`LEFT` or `RIGHT`) and rows.
struct Placed {
    side: usize,
    rows: Range<usize>,
}

impl Bands {
    /// The left edge and width of the text at `row`.
    fn at(&self, row: usize) -> (Pixels, Pixels) {
        let side = |s: usize| {
            if self
                .placed
                .iter()
                .any(|p| p.side == s && p.rows.contains(&row))
            {
                self.reserved[s]
            } else {
                px(0.)
            }
        };
        let (left, right) = (side(LEFT), side(RIGHT));
        (left, self.total - left - right)
    }

    /// How many rows from `row` on stay in the same band (`usize::MAX` once no
    /// float is left).
    fn rows_from(&self, row: usize) -> usize {
        self.placed
            .iter()
            .filter_map(|p| match row {
                r if r < p.rows.start => Some(p.rows.start - r),
                r if r < p.rows.end => Some(p.rows.end - r),
                _ => None,
            })
            .min()
            .unwrap_or(usize::MAX)
    }
}

/// One float of an aside, shaped at its own width, waiting to be put down.
struct FloatShape {
    side: usize,
    x: Pixels,
    width: Pixels,
    /// The shaped lines, from `y` 0.
    lines: Vec<ShapedLine>,
    height: Pixels,
}

/// The floats of an aside: each is placed where the text reaches the block it
/// was written in (an aside's all at its top), on its side below what is
/// already there.
struct Floats {
    shaped: Vec<FloatShape>,
    /// Where each side is free from, in pixels.
    side_y: [Pixels; 2],
    next: usize,
    bands: Bands,
    /// The placed floats' columns.
    columns: Vec<ShapedColumn>,
    /// The bottom row of the lowest float, air excluded.
    float_rows: usize,
}

impl Floats {
    /// Places every float written before `line` (the text lines come in
    /// order), the flow being at `y`.
    fn place_before(&mut self, region: &Columns, line: usize, y: Pixels, line_height: Pixels) {
        while self.next < self.shaped.len() && region.float_anchor(self.next) < line {
            self.place_next(y, line_height);
        }
    }

    /// Where every float so far has ended, air below it included.
    fn clear_y(&self) -> Pixels {
        self.side_y[LEFT].max(self.side_y[RIGHT])
    }

    fn place_next(&mut self, y: Pixels, line_height: Pixels) {
        let float = &mut self.shaped[self.next];
        self.next += 1;
        if float.height <= px(0.) {
            return;
        }
        let rows_of = |y: Pixels| (y / line_height).round() as usize;
        let top = y.max(self.side_y[float.side]);
        let mut lines = std::mem::take(&mut float.lines);
        for l in &mut lines {
            l.y += top;
        }
        // A row of air below a float, before text or the next float.
        self.side_y[float.side] = top + float.height + line_height;
        self.bands.placed.push(Placed {
            side: float.side,
            rows: rows_of(top)..rows_of(self.side_y[float.side]),
        });
        self.float_rows = self.float_rows.max(rows_of(top + float.height));
        self.columns.push(ShapedColumn {
            x: float.x,
            width: float.width,
            lines,
        });
    }
}

/// Lays out an aside: each float (parts `0..count - 1`) at its side's width,
/// floats on one side stacked with a row between, and the text (the last part)
/// flowing between them: each paragraph is as wide as the band its first row
/// is in, and where a band changes (a side's floats end) it is cut at a row
/// boundary and the rest continues in the new band. A table or callout that
/// starts in a band stays in it, so its box keeps one width.
fn shape_aside(
    text: &str,
    doc: &DocumentCache,
    (region, split): (&Columns, &columns::Split),
    (cursor, focused): (usize, bool),
    (font, font_size, line_height): (&Font, Pixels, Pixels),
    (colors, links): (InlineColors, Option<&LinkTable>),
    window: &mut Window,
) -> ShapedRegion {
    let total = px(split.total);
    let text_part = region.count - 1;
    let mut side_width = [px(0.); 2];
    let mut shaped_floats = Vec::with_capacity(text_part);
    for k in 0..text_part {
        let side = usize::from(split.float_is_right(k));
        let (x, width) = (px(split.x(k)), px(split.width(k)));
        side_width[side] = width;
        let mut lines = Vec::new();
        let mut y = px(0.);
        for i in region.column_lines(k) {
            let (results, broken, height) = shape_at(
                text,
                doc,
                i,
                width,
                (cursor, focused),
                (font, font_size, colors),
                (links, window),
            );
            lines.push(ShapedLine {
                line: i,
                y,
                height,
                results,
                broken,
                x,
                width,
                raw: 0..doc.line_end_byte(i) - doc.line_start_byte(i),
            });
            y += height;
        }
        shaped_floats.push(FloatShape {
            side,
            x,
            width,
            lines,
            height: y,
        });
    }
    let rows_of = |y: Pixels| (y / line_height).round() as usize;
    let mut floats = Floats {
        shaped: shaped_floats,
        side_y: [px(0.); 2],
        next: 0,
        bands: Bands {
            placed: Vec::new(),
            reserved: side_width.map(|w| {
                if w > px(0.) {
                    w + px(columns::GAP)
                } else {
                    px(0.)
                }
            }),
            total,
        },
        columns: Vec::with_capacity(region.count),
        float_rows: 0,
    };

    let mut flow_lines: Vec<ShapedLine> = Vec::new();
    let mut y = px(0.);
    let mut block_band: Option<((bool, usize), Pixels, Pixels)> = None;
    let mut clears = region.clears().iter().copied().peekable();
    for i in region.text_lines() {
        // A `::: clear` above this line: it starts below every float so far.
        while let Some(clear) = clears.next_if(|&c| c < i) {
            floats.place_before(region, clear, y, line_height);
            y = y.max(floats.clear_y());
        }
        floats.place_before(region, i, y, line_height);
        let row = rows_of(y);
        let block = table_index_at(doc.tables(), i)
            .map(|t| (true, t))
            .or_else(|| callout_index_at(doc.callouts(), i).map(|c| (false, c)));
        let (x, width) = match (block, block_band) {
            (Some(b), Some((id, x, w))) if b == id => (x, w),
            _ => floats.bands.at(row),
        };
        block_band = block.map(|b| (b, x, width));
        let (results, broken, height) = shape_at(
            text,
            doc,
            i,
            width,
            (cursor, focused),
            (font, font_size, colors),
            (links, window),
        );
        let range = doc.line_start_byte(i)..doc.line_end_byte(i);
        let len = range.len();
        let line_text = &text[range];
        let rows = rows_of(height);
        let state = column_state(doc, i, (cursor, focused), &broken);
        let continues = block.is_some();
        let cuttable = !continues
            && rows > floats.bands.rows_from(row)
            && results.len() == 1
            && flowable(&state, &results[0].1, line_text);
        if cuttable {
            let callout = state.callout;
            let mut rest = results[0].1.clone();
            let (mut base, mut at_row) = (0usize, row);
            loop {
                let (bx, bw) = floats.bands.at(at_row);
                let room = floats.bands.rows_from(at_row);
                let whole = shape_entry(
                    &rest,
                    Some(bw),
                    (font, font_size),
                    &colors,
                    (&broken_in(&broken, &(base..len)), callout),
                    false,
                    window,
                );
                let cut = (rows_of(whole.size(line_height).height) > room)
                    .then(|| justify::row_start(&whole, room))
                    .flatten()
                    .map(|at| rest.split_at(at))
                    .filter(|(head, tail, _)| {
                        !head.display_text.is_empty() && !tail.display_text.is_empty()
                    });
                let (piece, next, raw) = match cut {
                    Some((head, tail, raw_cut)) => (head, Some(tail), base..base + raw_cut),
                    None => (rest.clone(), None, base..len),
                };
                let piece_broken = broken_in(&broken, &raw);
                let wrapped = if next.is_some() {
                    shape_entry(
                        &piece,
                        Some(bw),
                        (font, font_size),
                        &colors,
                        (&piece_broken, callout),
                        false,
                        window,
                    )
                } else {
                    whole
                };
                let h = wrapped.size(line_height).height;
                flow_lines.push(ShapedLine {
                    line: i,
                    y,
                    height: h,
                    results: vec![(wrapped, piece)],
                    broken: piece_broken,
                    x: bx,
                    width: bw,
                    raw: raw.clone(),
                });
                y += h;
                let Some(tail) = next else {
                    break;
                };
                at_row += rows_of(h);
                base = raw.end;
                rest = tail;
            }
            continue;
        }
        flow_lines.push(ShapedLine {
            line: i,
            y,
            height,
            results,
            broken,
            x,
            width,
            raw: 0..len,
        });
        y += height;
    }
    // A float written after the last line of text goes below it.
    floats.place_before(region, usize::MAX, y, line_height);
    if clears.next().is_some() {
        y = y.max(floats.clear_y());
    }
    let flow_rows = rows_of(y);
    let mut columns = std::mem::take(&mut floats.columns);
    columns.push(ShapedColumn {
        x: px(split.x(text_part)),
        width: px(split.width(text_part)),
        lines: flow_lines,
    });
    columns.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    ShapedRegion {
        columns,
        rows: floats.float_rows.max(flow_rows).max(1),
        total,
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

/// The column holding `line`.
pub(super) fn column_with_line(shaped: &ShapedRegion, line: usize) -> Option<&ShapedColumn> {
    shaped
        .columns
        .iter()
        .find(|c| c.lines.iter().any(|l| l.line == line))
}

/// The window y of `line`'s top in `column`, whose region starts at
/// `region_top`; a line past the column's last is its bottom.
pub(super) fn line_top(column: &ShapedColumn, region_top: Pixels, line: usize) -> Pixels {
    let y = column.lines.iter().find(|sl| sl.line >= line).map_or_else(
        || column.lines.last().map_or(px(0.), |sl| sl.y + sl.height),
        |sl| sl.y,
    );
    region_top + y
}

/// Paints the region whose first line is at `top` below `bounds.origin.y`:
/// dividers, then per column the callout bands and table boxes (which go under
/// the text), then the lines.
pub(super) fn paint_region(
    env: &PaintEnv,
    shaped: &ShapedRegion,
    region: &Columns,
    (bounds, top): (Bounds<Pixels>, Pixels),
    (window, cx): (&mut Window, &mut App),
) {
    let doc = env.doc;
    let height = env.line_height * shaped.rows as f32;
    let divider = env.palette.color(Role::Border);
    // A float has no ruled divider: the text wraps around it.
    for pair in shaped.columns.windows(2).filter(|_| !region.is_aside()) {
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
    paint_region_controls(env, shaped, region, (bounds, top), (window, cx));
    // Text below a float uses the whole width, so an aside is masked as one.
    let whole = shaped
        .columns
        .first()
        .zip(shaped.columns.last())
        .map(|(a, b)| (a.x.min(b.x), (a.x + a.width).max(b.x + b.width)));
    for column in &shaped.columns {
        // Nothing a column paints may reach the next one (a table cannot
        // shrink below its columns' minimum); half the gap is left for the
        // table controls, which straddle a table's edge.
        let reach = px(columns::GAP / 2.);
        let (left, right) = match whole.filter(|_| region.is_aside()) {
            Some(span) => span,
            None => (column.x, column.x + column.width),
        };
        let mask = ContentMask {
            bounds: Bounds {
                origin: point(bounds.origin.x + left - reach, bounds.origin.y),
                size: size(right - left + reach * 2., bounds.size.height),
            },
        };
        window.with_content_mask(Some(mask), |window| {
            paint_column_blocks(env, column, bounds, top, (window, cx));
            let mut tables = None;
            for sl in column.lines.iter().filter(|sl| !sl.results.is_empty()) {
                let state = column_state(doc, sl.line, (env.cursor, env.show_cursor), &sl.broken);
                let line_start = doc.line_start_byte(sl.line);
                let range = sl.raw_in_doc(line_start);
                // The caret at the cut belongs to the rows below, not above it.
                let end = if sl.raw.end < doc.line_end_byte(sl.line) - line_start {
                    range.end.saturating_sub(1).max(range.start)
                } else {
                    range.end
                };
                let slot = LineSlot {
                    i: sl.line,
                    line_text: &env.text[range.clone()],
                    line_range: range.start..end,
                    state: &state,
                    results: &sl.results,
                    bounds: sl.frame(bounds),
                    y_offset: top + sl.y,
                    total_height: sl.height,
                };
                paint_line(env, &slot, &mut tables, window, cx);
            }
        });
    }
}

/// An aside's floats with the shares `percent` gives them.
pub(super) fn dragged_floats(region: &Columns, percent: &[u8]) -> Vec<columns::FloatSpec> {
    region
        .floats()
        .iter()
        .zip(percent)
        .map(|(f, &share)| columns::FloatSpec { share, ..*f })
        .collect()
}

/// The divider grips, `+` and unwrap while the pointer is over the region, or
/// the dragged grip and its drop line.
fn paint_region_controls(
    env: &PaintEnv,
    shaped: &ShapedRegion,
    region: &Columns,
    (bounds, top): (Bounds<Pixels>, Pixels),
    (window, cx): (&mut Window, &mut App),
) {
    let doc = env.doc;
    let Some(index) = doc.region_index_at(region.start_line) else {
        return;
    };
    let drag = env.column_drag.filter(|d| d.region == index);
    if drag.is_none() && env.column_hover != Some(index) {
        return;
    }
    let (Some(first), Some((_, split))) =
        (shaped.columns.first(), doc.side_by_side(region.start_line))
    else {
        return;
    };
    let width = f32::from(shaped.total);
    let height = f32::from(env.line_height * shaped.rows as f32);
    let frame = column_handles::Frame {
        dividers: (0..split.divider_count())
            .map(|k| split.divider_x(k))
            .collect(),
        width,
        height,
        can_add: region.count < columns::MAX_COLUMNS,
        buttons: !region.tail,
    };
    let origin = point(bounds.origin.x + first.x, bounds.origin.y + top);
    let at = |handle| {
        let (x, y, w, h) = column_handles::rect_of(&frame, handle);
        Bounds {
            origin: origin + point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    };
    let colors = handle_colors(env.palette, env.text_color);
    if let Some(drag) = drag {
        // Only the dragged grip, at the divider the shares would give.
        let moved_split = match region.kind {
            columns::Kind::Aside => {
                columns::aside_split(width, &dragged_floats(region, &drag.percent))
            }
            columns::Kind::Columns => columns::split(width, &drag.percent),
        };
        let Some(split) = moved_split else {
            return;
        };
        let moved = column_handles::Frame {
            dividers: (0..split.divider_count())
                .map(|k| split.divider_x(k))
                .collect(),
            ..frame
        };
        let handle = column_handles::Handle::Divider(drag.divider);
        let (x, y, w, h) = column_handles::rect_of(&moved, handle);
        let grip = Bounds {
            origin: origin + point(px(x), px(y)),
            size: size(px(w), px(h)),
        };
        let line = Bounds {
            origin: origin + point(px(moved.dividers[drag.divider]) - px(1.), px(0.)),
            size: size(px(2.), px(moved.height)),
        };
        window.paint_quad(fill(line, colors.accent));
        paint_control(
            grip,
            "icons/grip-vertical.svg",
            (false, true),
            &colors,
            (window, cx),
        );
        return;
    }
    for handle in column_handles::shown(&frame) {
        let (round, icon) = match handle {
            column_handles::Handle::Divider(_) => (false, "icons/grip-vertical.svg"),
            column_handles::Handle::Add => (true, "icons/plus.svg"),
            column_handles::Handle::Unwrap => (true, "icons/x.svg"),
        };
        paint_control(at(handle), icon, (round, false), &colors, (window, cx));
    }
}

/// The callout bands and table boxes of one column, from its lines' places.
fn paint_column_blocks(
    env: &PaintEnv,
    column: &ShapedColumn,
    bounds: Bounds<Pixels>,
    top: Pixels,
    (window, cx): (&mut Window, &mut App),
) {
    let doc = env.doc;
    let frame = Bounds {
        origin: bounds.origin,
        size: size(column.width, bounds.size.height),
    };
    let at = |line: usize| line_top(column, frame.origin.y + top, line);
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
            // A block is as wide as the line that opens it (text beside a float
            // is narrower than text below it).
            let span = (bounds.origin.x + sl.x, sl.width);
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
                sl.width,
                (env.font, env.font_size, env.text_color),
                window,
            );
            let table_left = bounds.origin.x + sl.x;
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
                table_left,
                &table_at,
                rows,
                env.palette,
                window,
            );
            let hovered = env.table_hover.filter(|(t, _)| *t == index);
            let dragging = env.table_drag.filter(|d| d.table == index);
            if hovered.is_some() || dragging.is_some() {
                let (grid, top) = grid_for(table, &columns, &table_at);
                let shown = hovered.map_or(&[][..], |(_, handles)| handles.as_slice());
                let colors = handle_colors(env.palette, env.text_color);
                let origin = point(table_left, top);
                paint_table_handles(&grid, origin, (shown, dragging), &colors, (window, cx));
            }
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
    let hit_entry = |sl: &ShapedLine, window: &mut Window| {
        let start = doc.line_start_byte(sl.line);
        let range = sl.raw_in_doc(start);
        let state = column_state(doc, sl.line, (cursor, focused), &sl.broken);
        let line_frame = HitFrame {
            bounds: sl.frame(frame.bounds),
            ..*frame
        };
        let found = hit_in_line(
            doc,
            (sl.line, &text[range.clone()], range.start),
            (&state, &sl.results),
            top + sl.y,
            &line_frame,
            window,
        );
        found.map_err(|_| range.end)
    };
    // The line under the pointer, wherever its column's edges are: text below a
    // float is wider than its column.
    let under = shaped
        .columns
        .iter()
        .flat_map(|c| &c.lines)
        .filter(|sl| !sl.results.is_empty())
        .find(|sl| {
            (sl.x..sl.x + sl.width).contains(&x)
                && (top + sl.y..top + sl.y + sl.height).contains(&frame.relative_y)
        });
    if let Some(sl) = under
        && let Ok(hit) = hit_entry(sl, window)
    {
        return hit;
    }
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
    let mut last = None;
    for sl in column.lines.iter().filter(|sl| !sl.results.is_empty()) {
        match hit_entry(sl, window) {
            Ok(hit) => return hit,
            Err(end) => last = Some(end),
        }
    }
    // Below the column's text, or an empty column: the end of its last line,
    // else the blank line right after the open marker.
    Hit::at(last.unwrap_or_else(|| doc.line_start_byte(region.start_line + 1)))
}

/// Where the caret at `offset` is on screen when it is in a column of a region
/// laid out side by side: the region's top is `region_top`, the editor's
/// bounds are `bounds`. `None` when `offset`'s line is in no such region.
pub(super) fn caret_bounds(
    text: &str,
    doc: &DocumentCache,
    offset: usize,
    (cursor, focused): (usize, bool),
    (font, font_size, line_height): (&Font, Pixels, Pixels),
    (colors, region_top, bounds): (InlineColors, Pixels, Bounds<Pixels>),
    window: &mut Window,
) -> Option<Bounds<Pixels>> {
    let line = doc.line_at(offset);
    let (region, split) = doc.side_by_side(line)?;
    let shaped = shape_region(
        text,
        doc,
        (&region, &split),
        (cursor, focused),
        (font, font_size, line_height),
        (colors, None),
        window,
    );
    let column = column_with_line(&shaped, line)?;
    let line_start = doc.line_start_byte(line);
    let within = offset - line_start;
    // A line cut around a float is two entries: the one holding the offset (the
    // later one when it is the cut itself), else the last.
    let pieces = || column.lines.iter().filter(|sl| sl.line == line);
    let sl = pieces()
        .find(|sl| sl.raw.start <= within && within < sl.raw.end)
        .or_else(|| pieces().last())?;
    for (wrapped, display) in &sl.results {
        let at = display.display_offset_for_raw(within.saturating_sub(sl.raw.start));
        let state = LineState::in_column(doc, line, cursor, focused);
        let Some(pos) = justify::position_for_index(
            (&state, wrapped, display),
            sl.results.len(),
            at,
            line_height,
        ) else {
            continue;
        };
        let x = pos.x + indent_px(display.indent.level());
        let origin = bounds.origin + point(sl.x + x, region_top + sl.y + pos.y);
        return Some(Bounds {
            origin,
            size: size(px(1.), line_height),
        });
    }
    None
}

/// Where `shaped` put its parts, for the paint trace (only built when tracing).
/// `top` is the region's first line's y below the editor's top.
pub(super) fn trace_geometry(
    env: &PaintEnv,
    shaped: &ShapedRegion,
    (start_line, top): (usize, Pixels),
    window: &mut Window,
) -> crate::text_editor::paint_trace::RegionGeom {
    use crate::text_editor::paint_trace::{ColumnGeom, RegionGeom};
    let doc = env.doc;
    let columns = shaped
        .columns
        .iter()
        .map(|column| {
            let rows = |sl: &ShapedLine| (sl.height / env.line_height).round() as usize;
            let lines = column
                .lines
                .iter()
                .filter(|sl| !sl.results.is_empty())
                .map(|sl| {
                    (
                        sl.line,
                        f32::from(top + sl.y),
                        rows(sl),
                        f32::from(sl.height),
                    )
                })
                .collect();
            let bottom = column.lines.last().map_or(top, |sl| top + sl.y + sl.height);
            let mut tables = Vec::new();
            let mut last = None;
            for sl in &column.lines {
                let Some(index) = table_index_at(doc.tables(), sl.line) else {
                    continue;
                };
                if last == Some(index)
                    || is_table_active(doc, doc.tables()[index].start_line, env.show_cursor)
                {
                    continue;
                }
                last = Some(index);
                let table = &doc.tables()[index];
                let boxed = TableColumns::of(
                    table,
                    column.width,
                    (env.font, env.font_size, env.text_color),
                    window,
                );
                let first = line_top(column, top, table.start_line);
                let end = line_top(column, top, table.end_line);
                let width = boxed.x(table_box::box_width(&boxed.widths));
                tables.push((
                    table.start_line,
                    f32::from(first),
                    f32::from(width),
                    f32::from(end - first),
                ));
            }
            ColumnGeom {
                x: f32::from(column.x),
                width: f32::from(column.width),
                bottom: f32::from(bottom),
                lines,
                tables,
            }
        })
        .collect();
    RegionGeom {
        start_line,
        top: f32::from(top),
        height: f32::from(env.line_height * shaped.rows as f32),
        columns,
    }
}
